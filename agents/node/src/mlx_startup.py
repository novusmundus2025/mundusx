"""Report actual completed weight evaluation; never generate qualification tokens."""
import json
import os
from pathlib import Path
import threading
import re

PREFIX = "MUNDUSX_LOAD_PROGRESS:"
BATCH_BYTES = 64 * 1024 * 1024


class RequiredToolPrefix:
    """Per-request opening only: never supplies a tool name or arguments."""
    def __init__(self, tokens):
        if not tokens:
            raise ValueError("Required tool prefix must not be empty")
        self.tokens = tuple(tokens)
        self.start = None

    def next_token(self, input_length):
        if self.start is None:
            self.start = input_length
        offset = input_length - self.start
        if offset < 0:
            raise ValueError("Required tool token history moved backwards")
        return self.tokens[offset] if offset < len(self.tokens) else None


class RegisteredToolOpening:
    """Constrain only the opening and registered function name, never values."""
    def __init__(self, sequences):
        if not sequences or any(not sequence for sequence in sequences):
            raise ValueError("Registered tool openings must not be empty")
        self.sequences = tuple(tuple(sequence) for sequence in sequences)
        self.start = None

    def next_tokens(self, history):
        if self.start is None:
            self.start = len(history)
        generated = tuple(history[self.start:])
        candidates = [sequence for sequence in self.sequences if sequence[:len(generated)] == generated]
        if not candidates:
            raise ValueError("Generated tool opening left the registered tool set")
        if any(len(sequence) == len(generated) for sequence in candidates):
            return None
        return sorted({sequence[len(generated)] for sequence in candidates})


class AutomaticToolOpening:
    """Free prose until a tool marker; then only a registered function name."""
    def __init__(self, marker, sequences):
        if not marker:
            raise ValueError("Tool marker must not be empty")
        self.marker = tuple(marker)
        self.prefix = RegisteredToolOpening(sequences)
        self.start = None
        self.scanned = 0
        self.finished = False

    def next_tokens(self, history):
        if self.start is None:
            self.start = len(history)
        if self.finished:
            return None
        if self.prefix.start is None:
            generated = history[self.start:]
            for index in range(max(0, self.scanned - len(self.marker) + 1), len(generated) - len(self.marker) + 1):
                if tuple(generated[index:index + len(self.marker)]) == self.marker:
                    self.prefix.start = self.start + index
                    break
            self.scanned = len(generated)
            if self.prefix.start is None:
                return None
        tokens = self.prefix.next_tokens(history)
        if tokens is None:
            self.finished = True
        return tokens


def repetitive_tool_tail(text):
    """Reject long low-entropy argument loops, not ordinary repeated syntax."""
    if '<tool_call>' not in text or text.rfind('<tool_call>') < text.rfind('</tool_call>'):
        return False
    lines=[line.strip() for line in text[text.rfind('<tool_call>'):].splitlines() if line.strip()][-24:]
    growing=0
    for previous,current in zip(lines,lines[1:]):
        growing=growing+1 if len(previous)>=8 and current.startswith(previous) and 0<len(current)-len(previous)<=16 else 0
        if growing>=6:
            return True
    tail = re.sub(r'\s+', '', text[text.rfind('<tool_call>'):])[-1024:]
    if len(tail) < 192:
        return False
    return any(tail.endswith(tail[-width:] * max(4, 192 // width)) for width in range(1, 257))


class NativeToolGuard:
    """Stop only at a complete call or a rejected repetitive argument tail."""
    def __init__(self, tokenizer):
        self.tokenizer = tokenizer
        self.start = None
        self.end = tuple(tokenizer.encode('</tool_call>', add_special_tokens=False))

    def stop(self, history):
        if self.start is None:
            self.start = len(history)
        generated = history[self.start:]
        if self.end and tuple(generated[-len(self.end):]) == self.end:
            return True
        if len(generated) >= 64 and len(generated) % 16 == 0:
            return repetitive_tool_tail(self.tokenizer.decode(generated))
        return False


def stable_host_prefix(tokenizer, request, template_args, tokenized):
    """Add a cache boundary without changing a single rendered prompt token.

    Recurrent caches cannot trim a previous longer prompt. Retain the stable
    host instructions separately from changing tool catalogs and observations,
    including requests whose final message is a system or tool message.
    """
    # MLX versions with vision support append a fifth item. Preserve all
    # runtime metadata and leave multimodal prompts untouched.
    if len(tokenized) < 4 or (len(tokenized) > 4 and tokenized[4] is not None):
        return tokenized
    prompt, segments, segment_types, initial_state = tokenized[:4]
    if (request.request_type != 'chat' or not request.messages
            or request.messages[0].get('role') != 'system'
            or not getattr(tokenizer, 'has_chat_template', False)
            or not getattr(getattr(tokenizer, 'tool_parser', None), '__module__', '').endswith('qwen3_coder')):
        return tokenized
    try:
        options = dict(template_args)
        options.update(tools=[], tokenize=True)
        baseline = tokenizer.apply_chat_template(
            [request.messages[0], {'role': 'user', 'content': ''}],
            add_generation_prompt=False, **options)
        boundary = 0
        for actual, expected in zip(prompt, baseline):
            if actual != expected:
                break
            boundary += 1
    except (TypeError, ValueError, AttributeError):
        return tokenized
    # Tiny prefixes do not justify an extra recurrent-state cache entry.
    if boundary < 256 or boundary >= len(prompt):
        return tokenized
    boundaries = {boundary: 'system'}
    position = 0
    for segment, kind in zip(segments, segment_types):
        position += len(segment)
        if position > boundary:
            boundaries[position] = 'user' if kind == 'system' else kind
    if position != len(prompt):
        return tokenized
    result, kinds, start = [], [], 0
    for end, kind in sorted(boundaries.items()):
        result.append(prompt[start:end])
        kinds.append(kind)
        start = end
    return (prompt, result, kinds, initial_state, *tokenized[4:])


def install_stable_host_cache(server):
    original = server.ResponseGenerator._tokenize

    def tokenize(self, tokenizer, request, args):
        tokenized = original(self, tokenizer, request, args)
        template_args = dict(self.model_provider.cli_args.chat_template_args)
        template_args.update(args.chat_template_kwargs or {})
        return stable_host_prefix(tokenizer, request, template_args, tokenized)

    server.ResponseGenerator._tokenize = tokenize


def install_required_tool_opening(server, mx):
    original_tokenize = server.ResponseGenerator._tokenize
    original_processors = server._make_logits_processors

    def tokenize(self, tokenizer, request, args):
        options = args.chat_template_kwargs or {}
        guarded = options.get('opengpu_native_tool_guard') is True
        if guarded:
            if not tokenizer.tool_parser.__module__.endswith('qwen3_coder') or not tokenizer.eos_token_ids:
                raise ValueError('Native tool guard needs Qwen3-Coder with EOS tokens')
            args._opengpu_native_guard = (tokenizer, min(tokenizer.eos_token_ids))
        required = options.get("opengpu_require_tool_call") is True
        if required:
            if request.request_type != "chat" or not request.tools or not tokenizer.has_tool_calling:
                raise ValueError("Required tool opening needs a tool-capable chat request")
            if not tokenizer.tool_parser.__module__.endswith("qwen3_coder"):
                raise ValueError("Required tool opening supports the Qwen3-Coder parser only")
            names = [tool.get("function", {}).get("name") for tool in request.tools]
            if any(not isinstance(name, str) or not name for name in names):
                raise ValueError("Required tool opening needs registered function names")
            args._opengpu_required_openings = tuple(tuple(tokenizer.encode("<tool_call>\n<function=" + name + ">\n", add_special_tokens=False)) for name in names)
        elif guarded and request.tools:
            names = [tool.get("function", {}).get("name") for tool in request.tools]
            if any(not isinstance(name, str) or not name for name in names):
                raise ValueError("Automatic tool opening needs registered function names")
            args._opengpu_auto_openings = (
                tuple(tokenizer.encode('<tool_call>', add_special_tokens=False)),
                tuple(tuple(tokenizer.encode('<tool_call>\n<function=' + name + '>\n', add_special_tokens=False)) for name in names),
            )
        args.chat_template_kwargs = {key: value for key, value in options.items() if key not in ('opengpu_require_tool_call', 'opengpu_native_tool_guard')}
        return original_tokenize(self, tokenizer, request, args)

    def processors(args):
        result = list(original_processors(args))
        openings = getattr(args, "_opengpu_required_openings", None)
        if openings:
            prefix = RegisteredToolOpening(openings)
            finished = False

            def force_opening(history, logits):
                nonlocal finished
                if finished:
                    return logits
                tokens = prefix.next_tokens(history.tolist())
                if tokens is None:
                    finished = True
                    return logits
                allowed = mx.any(mx.arange(logits.shape[-1])[:, None] == mx.array(tokens)[None, :], axis=1)
                return mx.where(allowed, logits, mx.full_like(logits, -float("inf")))

            result.append(force_opening)
        automatic = getattr(args, '_opengpu_auto_openings', None)
        if automatic:
            prefix = AutomaticToolOpening(*automatic)
            def guard_opening(history, logits):
                tokens = prefix.next_tokens(history.tolist())
                if tokens is None:
                    return logits
                allowed = mx.any(mx.arange(logits.shape[-1])[:, None] == mx.array(tokens)[None, :], axis=1)
                return mx.where(allowed, logits, mx.full_like(logits, -float('inf')))
            result.append(guard_opening)
        guarded = getattr(args, '_opengpu_native_guard', None)
        if guarded:
            tokenizer, eos = guarded
            guard = NativeToolGuard(tokenizer)
            def stop_invalid_or_complete(history, logits):
                if not guard.stop(history.tolist()):
                    return logits
                return mx.where(mx.arange(logits.shape[-1]) == eos, logits, mx.full_like(logits, -float('inf')))
            result.append(stop_invalid_or_complete)
        return result

    server.ResponseGenerator._tokenize = tokenize
    server._make_logits_processors = processors


def install_capabilities_endpoint(server):
    """Let a new agent distinguish a patched running server from an old one."""
    original_get = server.APIHandler.do_GET

    def get(self):
        if self.path != "/v1/opengpu/runtime-capabilities":
            return original_get(self)
        payload = json.dumps({"required_tool_opening": True, "native_tool_guard": True, "registered_auto_tool_opening": True, "stable_host_prefix_cache": True, "version": 10}).encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    server.APIHandler.do_GET = get


def complete_qwen_call_parser(parser):
    """Keep complete calls when Qwen adds prose after the closing function."""
    def parse(text, tools=None):
        matches = list(re.finditer(r"<function=[^<>\s]+>[\s\S]*?</function>", text))
        if len(matches) != 1:
            raise ValueError("Expected one complete Qwen function block")
        match = matches[0]
        block = match.group(0)
        if block.count("<function=") != 1 or block.count("<parameter=") != block.count("</parameter>"):
            raise ValueError("Nested or incomplete Qwen function or parameter blocks")
        remainder = text[:match.start()] + text[match.end():]
        if re.search(r"</?(?:function|parameter|tool_call)(?:[=>\s]|$)", remainder):
            raise ValueError("Incomplete or ambiguous extra Qwen tool markup")
        # The original schema-aware parser still converts every argument.
        # Never synthesize missing values, closing tags, or additional calls.
        return parser(block, tools)
    return parse


def install_complete_qwen_parser(server):
    original_init = server.ToolCallFormatter.__init__
    local = threading.local()

    def initialize(self, tool_parser, tools, streaming=False):
        if getattr(tool_parser, "__module__", "").endswith("qwen3_coder"):
            strict = complete_qwen_call_parser(tool_parser)
            def checked(text, tools=None):
                try:
                    return strict(text, tools)
                except (ValueError, json.JSONDecodeError):
                    if getattr(local, 'active', False):
                        local.error = 'repetitive tool argument' if repetitive_tool_tail('<tool_call>' + text) else 'incomplete or ambiguous tool markup'
                    raise
            tool_parser = checked
        original_init(self, tool_parser, tools, streaming)

    server.ToolCallFormatter.__init__ = initialize
    if not hasattr(server, 'APIHandler'):
        return
    original_handle = server.APIHandler.handle_completion
    original_response = server.APIHandler.generate_response
    def handle(self, *args, **kwargs):
        local.active = (self.body.get('chat_template_kwargs') or {}).get('opengpu_native_tool_guard') is True
        local.error = None
        try:
            return original_handle(self, *args, **kwargs)
        finally:
            local.active = False
            local.error = None
    def response(self, *args, **kwargs):
        value = original_response(self, *args, **kwargs)
        if getattr(local, 'active', False) and getattr(local, 'error', None):
            value['choices'][0]['opengpu_tool_error'] = local.error
        return value
    server.APIHandler.handle_completion = handle
    server.APIHandler.generate_response = response


def bounded_prompt_cache(cache_class, max_bytes):
    """Cap retained prefixes on every insertion, including non-batched requests."""
    if max_bytes < 0:
        raise ValueError("Prompt cache byte budget must not be negative")

    class BoundedPromptCache(cache_class):
        def __init__(self, max_size=2, **kwargs):
            kwargs["max_bytes"] = min(kwargs.get("max_bytes", max_bytes), max_bytes)
            super().__init__(max_size=max_size, **kwargs)

    return BoundedPromptCache


def leaves(value):
    if isinstance(value, dict):
        for item in value.values():
            yield from leaves(item)
    elif isinstance(value, (list, tuple)):
        for item in value:
            yield from leaves(item)
    elif isinstance(getattr(value, "nbytes", None), int):
        yield value


def evaluate_weights(evaluate, parameters, report, batch_bytes=BATCH_BYTES):
    arrays = list({id(array): array for array in leaves(parameters)}.values())
    total = sum(array.nbytes for array in arrays)
    if not total:
        return evaluate(parameters)
    report(0, total)
    batch, size, completed = [], 0, 0
    result = None
    for array in arrays:
        batch.append(array)
        size += array.nbytes
        if size >= batch_bytes:
            result = evaluate(*batch)
            completed += size
            report(completed, total)
            batch, size = [], 0
    if batch:
        result = evaluate(*batch)
        completed += size
        report(completed, total)
    return result


def main():
    import mlx.core as mx
    import mlx_lm.utils as utils
    import mlx_lm.models.cache as cache_module

    cache_module.LRUPromptCache = bounded_prompt_cache(
        cache_module.LRUPromptCache,
        int(os.environ["OPENGPU_MLX_PROMPT_CACHE_BYTES"]),
    )

    state = threading.local()
    original_eval = mx.eval
    original_load = utils.load
    ready = Path(os.environ["OPENGPU_MLX_READY_FILE"])
    startup_complete = False

    def report(completed, total):
        print(PREFIX + json.dumps({"completed_bytes": completed, "total_bytes": total}), flush=True)

    def measured_eval(*args, **kwargs):
        if getattr(state, "loading", False) and len(args) == 1 and isinstance(args[0], dict) and not kwargs:
            return evaluate_weights(original_eval, args[0], report)
        return original_eval(*args, **kwargs)

    def measured_load(*args, **kwargs):
        nonlocal startup_complete
        previous = getattr(state, "loading", False)
        state.loading = not startup_complete
        try:
            result = original_load(*args, **kwargs)
            if not startup_complete and not kwargs.get("lazy", False):
                ready.write_text("ready", encoding="utf-8")
                startup_complete = True
            return result
        except Exception as error:
            if not startup_complete:
                ready.with_suffix(".error").write_text(str(error)[:4096], encoding="utf-8")
            raise
        finally:
            state.loading = previous

    mx.eval = measured_eval
    utils.load = measured_load
    import mlx_lm.server as server
    install_required_tool_opening(server, mx)
    install_stable_host_cache(server)
    install_complete_qwen_parser(server)
    install_capabilities_endpoint(server)
    server.main()


if __name__ == "__main__":
    main()
