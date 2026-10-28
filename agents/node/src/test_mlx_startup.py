import unittest
import importlib.util
from mlx_startup import evaluate_weights, bounded_prompt_cache, RequiredToolPrefix, RegisteredToolOpening, AutomaticToolOpening, install_required_tool_opening, install_capabilities_endpoint, complete_qwen_call_parser, install_complete_qwen_parser, repetitive_tool_tail, NativeToolGuard
from types import SimpleNamespace
from mlx_startup import stable_host_prefix


class StableHostCacheTests(unittest.TestCase):
    def test_vision_runtime_tuple_preserves_metadata(self):
        def parser(): pass
        parser.__module__ = 'mlx_lm.tool_parsers.qwen3_coder'
        prefix = list(range(300))
        tokenizer = SimpleNamespace(has_chat_template=True, tool_parser=parser,
            apply_chat_template=lambda *a, **k: prefix + [999])
        request = SimpleNamespace(request_type='chat', messages=[{'role':'system','content':'s'}])
        original = (prefix + [800], [prefix + [800]], ['assistant'], 'normal', None)
        result = stable_host_prefix(tokenizer, request, {}, original)
        self.assertEqual(len(result), 5)
        self.assertIsNone(result[4])
        self.assertEqual(sum(result[1], []), original[0])
        vision = object()
        multimodal = (*original[:4], vision)
        self.assertIs(stable_host_prefix(tokenizer, request, {}, multimodal), multimodal)

    def test_system_and_tool_endings_keep_exact_tokens_and_thinking_tail(self):
        def parser(): pass
        parser.__module__ = 'mlx_lm.tool_parsers.qwen3_coder'
        prefix = list(range(300))
        calls = []
        def template(messages, **kwargs):
            calls.append((messages, kwargs))
            return prefix + [900, 901]
        tokenizer = SimpleNamespace(has_chat_template=True, tool_parser=parser,
                                    apply_chat_template=template)
        for role in ['user', 'system', 'tool']:
            request = SimpleNamespace(request_type='chat', messages=[
                {'role':'system','content':'stable host'}, {'role':role,'content':'observation'}])
            prompt = prefix + [800, 801, 802]
            original = (prompt, [prompt[:-1], prompt[-1:]], ['system','assistant'], 'reasoning')
            result = stable_host_prefix(tokenizer, request, {'enable_thinking':False}, original)
            self.assertIs(result[0], prompt)
            self.assertEqual(result[1], [prefix, [800,801], [802]])
            self.assertEqual(result[2], ['system','user','assistant'])
            self.assertEqual(result[3], 'reasoning')
            self.assertEqual(sum(result[1], []), prompt)
            self.assertEqual(original[1], [prompt[:-1], prompt[-1:]])
        self.assertEqual(calls[0][1], {'enable_thinking':False,'tools':[],
                                     'tokenize':True,'add_generation_prompt':False})

    def test_unsupported_and_small_prefixes_keep_original_segmentation(self):
        def parser(): pass
        parser.__module__ = 'mlx_lm.tool_parsers.qwen3_coder'
        tokenizer = SimpleNamespace(has_chat_template=True,tool_parser=parser,
            apply_chat_template=lambda *a,**k: [1,2])
        request = SimpleNamespace(request_type='chat',messages=[{'role':'system','content':'s'}])
        original = ([1,3], [[1,3]], ['assistant'], 'normal')
        self.assertIs(stable_host_prefix(tokenizer,request,{},original), original)
        request.request_type='completion'
        self.assertIs(stable_host_prefix(tokenizer,request,{},original), original)


class Array:
    def __init__(self, size):
        self.nbytes = size
        self.done = False


class ProgressTests(unittest.TestCase):
    def test_argument_loop_detection_does_not_treat_ordinary_code_as_a_loop(self):
        self.assertTrue(repetitive_tool_tail('<tool_call><function=create_file><parameter=content>' + 'coverage/\n' * 35))
        self.assertTrue(repetitive_tool_tail('<tool_call><function=create_file><parameter=content>' + ('coverage/lcov-report/*.yml\ncoverage/lcov-report/*.yaml\n' * 10)))
        self.assertTrue(repetitive_tool_tail('<tool_call><function=create_file><parameter=content>' + (''.join('coverage/lcov-report/coverage-summary.'+extension+'\n' for extension in ['json','txt','xml']) * 10)))
        self.assertTrue(repetitive_tool_tail('<tool_call><function=create_file><parameter=content>\n'+'\n'.join('coverage/**/*.*.cache'+'.*'*i for i in range(1,12))))
        self.assertTrue(repetitive_tool_tail('<tool_call><function=create_file><parameter=content>' + 'coverage/**/*.' + '*.' * 150))
        self.assertFalse(repetitive_tool_tail('coverage/\n' * 100))
        self.assertFalse(repetitive_tool_tail('<tool_call><function=read_file></function></tool_call>' + 'coverage/\n' * 100))
        self.assertFalse(repetitive_tool_tail('<tool_call><function=create_file><parameter=content>' + '\n'.join(f'function f{i}() {{ return {i}; }}' for i in range(100))))

    def test_guard_finishes_one_call_and_keeps_requests_independent(self):
        tokenizer = SimpleNamespace(encode=lambda *a, **kw: [8, 9], decode=lambda ids: ''.join(chr(i) for i in ids))
        first, second = NativeToolGuard(tokenizer), NativeToolGuard(tokenizer)
        self.assertFalse(first.stop([1, 2]))
        self.assertFalse(second.stop([3]))
        self.assertFalse(first.stop([1, 2, 8]))
        self.assertTrue(first.stop([1, 2, 8, 9]))
        self.assertFalse(second.stop([3, 7]))

    def test_rejected_runtime_markup_is_an_explicit_private_error(self):
        class Formatter:
            def __init__(self, parser, tools, streaming=False): self.parser=parser
        def parser(text, tools): return {'name':'read','arguments':{}}
        parser.__module__='mlx_lm.tool_parsers.qwen3_coder'
        class Handler:
            def handle_completion(self):
                try: Formatter(parser, []).parser('<function=read>', [])
                except ValueError: pass
                return self.generate_response()
            def generate_response(self): return {'choices':[{'message':{'content':'I will read'}}]}
        install_complete_qwen_parser(SimpleNamespace(ToolCallFormatter=Formatter,APIHandler=Handler))
        handler=Handler();handler.body={'chat_template_kwargs':{'opengpu_native_tool_guard':True}}
        self.assertIn('opengpu_tool_error',handler.handle_completion()['choices'][0])
        handler.body={}
        self.assertNotIn('opengpu_tool_error',handler.handle_completion()['choices'][0])

    def test_capability_endpoint_is_narrow_and_does_not_handle_other_routes(self):
        import io
        class Handler:
            def do_GET(self):
                return "original route"
            def send_response(self, status):
                self.status = status
            def send_header(self, name, value):
                pass
            def end_headers(self):
                pass
        install_capabilities_endpoint(SimpleNamespace(APIHandler=Handler))
        handler = Handler()
        handler.path = "/v1/opengpu/runtime-capabilities"
        handler.wfile = io.BytesIO()
        handler.do_GET()
        self.assertEqual(handler.status, 200)
        import json
        self.assertEqual(json.loads(handler.wfile.getvalue()), {"required_tool_opening": True, "native_tool_guard": True, "registered_auto_tool_opening": True, "stable_host_prefix_cache": True, "version": 10})
        handler.path = "/v1/models"
        self.assertEqual(handler.do_GET(), "original route")

    def test_automatic_names_allow_prose_and_leave_argument_values_unconstrained(self):
        a=AutomaticToolOpening([1,2], [[1,2,3,4], [1,2,3,5]])
        b=AutomaticToolOpening([1,2], [[1,2,3,9]])
        self.assertIsNone(a.next_tokens([100,101]))
        self.assertIsNone(b.next_tokens([200]))
        self.assertIsNone(a.next_tokens([100,101,77,1]))
        self.assertEqual(a.next_tokens([100,101,77,1,2]),[3])
        self.assertEqual(a.next_tokens([100,101,77,1,2,3]),[4,5])
        self.assertIsNone(a.next_tokens([100,101,77,1,2,3,5]))
        self.assertIsNone(a.next_tokens([100,101,77,1,2,3,5,999,888]))
        self.assertIsNone(b.next_tokens([200,66]))
        self.assertEqual(b.next_tokens([200,66,1,2,3]),[9])
        with self.assertRaises(ValueError):AutomaticToolOpening([],[[1]])

    def test_registered_name_choice_branches_and_isolates_requests(self):
        first = RegisteredToolOpening([[1, 2, 3], [1, 4, 3]])
        second = RegisteredToolOpening([[1, 9, 3]])
        self.assertEqual(first.next_tokens([100]), [1])
        self.assertEqual(second.next_tokens([200, 201]), [1])
        self.assertEqual(first.next_tokens([100, 1]), [2, 4])
        self.assertEqual(first.next_tokens([100, 1, 4]), [3])
        self.assertIsNone(first.next_tokens([100, 1, 4, 3]))
        self.assertEqual(second.next_tokens([200, 201, 1]), [9])
        with self.assertRaises(ValueError):
            second.next_tokens([200, 201, 1, 8])

    @unittest.skipUnless(importlib.util.find_spec("mlx"), "native MLX checked on macOS release runner")
    def test_complete_qwen_call_survives_trailing_prose_with_exact_typed_arguments(self):
        from mlx_lm.tool_parsers.qwen3_coder import parse_tool_call
        tools = [{"type": "function", "function": {"name": "read_file", "parameters": {"properties": {"filePath": {"type": "string"}, "startLine": {"type": "number"}, "endLine": {"type": "number"}}}}}]
        raw = '<function=read_file>\n<parameter=filePath>\nD:\\sample\\test.js\n</parameter>\n<parameter=startLine>\n1\n</parameter>\n<parameter=endLine>\n15\n</parameter>\n</function>\n Kyle\n'
        parsed = complete_qwen_call_parser(parse_tool_call)(raw, tools)
        self.assertEqual(parsed, {"name": "read_file", "arguments": {"filePath": "D:\\sample\\test.js", "startLine": 1, "endLine": 15}})
        self.assertEqual(complete_qwen_call_parser(parse_tool_call)(raw[:-6], tools), parsed)
        for invalid in [raw.replace('</function>', ''), raw + '<function=other>', raw + '<function=other></function>', '<function=first><function=second></function> trailing', raw.replace('15', 'not-a-number')]:
            with self.assertRaises(ValueError):
                complete_qwen_call_parser(parse_tool_call)(invalid, tools)

    def test_qwen_recovery_never_invents_a_missing_block_or_argument(self):
        def strict(text, tools):
            if text != '<function=read></function>':
                raise ValueError('No function provided')
            return {"name": "read", "arguments": {}}
        parser = complete_qwen_call_parser(strict)
        self.assertEqual(parser('<function=read></function> prose'), {"name": "read", "arguments": {}})
        for text in ['', 'just prose', '<function=read>', '<function=read></function><parameter=x>', '<function=read></function><function=next>']:
            with self.assertRaises(ValueError):
                parser(text)

    def test_other_model_parsers_are_not_changed(self):
        class Formatter:
            def __init__(self, parser, tools, streaming=False):
                self.parser = parser
        def original(text, tools):
            return text
        original.__module__ = 'other_model'
        install_complete_qwen_parser(SimpleNamespace(ToolCallFormatter=Formatter))
        self.assertIs(Formatter(original, []).parser, original)

    def test_required_tool_openings_are_bounded_and_independent_per_request(self):
        first, second = RequiredToolPrefix([1, 2, 3]), RequiredToolPrefix([1, 2, 3])
        self.assertEqual(first.next_token(100), 1)
        self.assertEqual(second.next_token(200), 1)
        self.assertEqual(first.next_token(101), 2)
        self.assertEqual(second.next_token(201), 2)
        self.assertEqual(first.next_token(102), 3)
        self.assertIsNone(first.next_token(103))
        self.assertEqual(second.next_token(202), 3)
        with self.assertRaises(ValueError):
            first.next_token(99)
        with self.assertRaises(ValueError):
            RequiredToolPrefix([])

    @unittest.skipUnless(importlib.util.find_spec("mlx"), "native MLX checked on macOS release runner")
    def test_automatic_opening_hook_preserves_free_text_and_masks_only_registered_name(self):
        import mlx.core as mx
        class Generator:
            def _tokenize(self, tokenizer, request, args): return "prompt"
        def parser(*_): pass
        parser.__module__="mlx_lm.tool_parsers.qwen3_coder"
        tokenizer=SimpleNamespace(has_tool_calling=True,tool_parser=parser,eos_token_ids={7},encode=lambda text,**kw: [1] if text=='<tool_call>' else ([1,2,3] if text.startswith('<tool_call>') else [6]),decode=lambda ids: '')
        server=SimpleNamespace(ResponseGenerator=Generator,_make_logits_processors=lambda _: [])
        install_required_tool_opening(server,mx)
        request=SimpleNamespace(request_type='chat',tools=[{'function':{'name':'registered'}}])
        args=SimpleNamespace(chat_template_kwargs={'opengpu_native_tool_guard':True})
        self.assertEqual(Generator()._tokenize(tokenizer,request,args),'prompt')
        processors=server._make_logits_processors(args);self.assertEqual(len(processors),2)
        logits=mx.arange(8).astype(mx.float32);guard=processors[0]
        self.assertIs(guard(mx.array([100,101]),logits),logits)
        self.assertIs(guard(mx.array([100,101,55]),logits),logits)
        masked=guard(mx.array([100,101,55,1]),logits)
        self.assertEqual(mx.argmax(masked).item(),2);self.assertEqual(mx.sum(mx.isfinite(masked)).item(),1)
        self.assertEqual(mx.argmax(guard(mx.array([100,101,55,1,2]),logits)).item(),3)
        self.assertIs(guard(mx.array([100,101,55,1,2,3]),logits),logits)
        self.assertIs(guard(mx.array([100,101,55,1,2,3,44]),logits),logits)

    @unittest.skipUnless(importlib.util.find_spec("mlx"), "native MLX checked on macOS release runner")
    def test_required_opening_masks_only_prefix_tokens_and_leaves_chat_unchanged(self):
        import mlx.core as mx
        class Generator:
            def _tokenize(self, tokenizer, request, args):
                self.seen_options = args.chat_template_kwargs
                return "original prompt"
        def parser(*_):
            pass
        parser.__module__ = "mlx_lm.tool_parsers.qwen3_coder"
        tokenizer = SimpleNamespace(has_tool_calling=True, tool_parser=parser, encode=lambda text, **kwargs: [1, 2, 3])
        server = SimpleNamespace(ResponseGenerator=Generator, _make_logits_processors=lambda _: [])
        install_required_tool_opening(server, mx)
        request = SimpleNamespace(request_type="chat", tools=[{"type": "function", "function": {"name": "test"}}])
        args = SimpleNamespace(chat_template_kwargs={"opengpu_require_tool_call": True, "enable_thinking": False})
        generator = Generator()
        self.assertEqual(generator._tokenize(tokenizer, request, args), "original prompt")
        self.assertEqual(generator.seen_options, {"enable_thinking": False})
        processors = server._make_logits_processors(args)
        self.assertEqual(len(processors), 1)
        logits = mx.arange(8).astype(mx.float32)
        for length, expected in [(10, 1), (11, 2), (12, 3)]:
            masked = processors[0](mx.array(list(range(10)) + [1, 2, 3][:length-10]), logits)
            self.assertEqual(mx.argmax(masked).item(), expected)
            self.assertEqual(mx.sum(mx.isfinite(masked)).item(), 1)
        self.assertIs(processors[0](mx.array(list(range(10)) + [1, 2, 3]), logits), logits)
        self.assertIs(processors[0](mx.arange(20), logits), logits)
        plain = SimpleNamespace(chat_template_kwargs=None)
        generator._tokenize(tokenizer, request, plain)
        self.assertEqual(server._make_logits_processors(plain), [])
        request.tools = []
        with self.assertRaisesRegex(ValueError, "tool-capable"):
            generator._tokenize(tokenizer, request, SimpleNamespace(chat_template_kwargs={"opengpu_require_tool_call": True}))

    @unittest.skipUnless(importlib.util.find_spec("mlx"), "native MLX checked on macOS release runner")
    def test_prefix_cache_is_bounded_and_sessions_get_independent_copies(self):
        import mlx.core as mx
        from mlx_lm.models.cache import LRUPromptCache, KVCache
        mx.set_default_device(mx.cpu)
        def entry():
            cache = KVCache()
            cache.update_and_fetch(mx.ones((1, 1, 4, 2)), mx.ones((1, 1, 4, 2)))
            return cache
        size = entry().nbytes
        cache = bounded_prompt_cache(LRUPromptCache, size * 2)(max_size=2)
        for tokens in ([1, 2, 3], [4, 5, 6], [7, 8, 9]):
            cache.insert_cache('model', tokens, [entry()])
            self.assertLessEqual(cache.nbytes, size * 2)
            self.assertLessEqual(len(cache), 2)
        first, rest = cache.fetch_nearest_cache('model', [7, 8, 9])
        self.assertEqual(rest, [])
        first[0].update_and_fetch(mx.ones((1, 1, 1, 2)), mx.ones((1, 1, 1, 2)))
        second, _ = cache.fetch_nearest_cache('model', [7, 8, 9])
        self.assertEqual(second[0].offset, 4)
        too_small = bounded_prompt_cache(LRUPromptCache, 1)(max_size=2)
        too_small.insert_cache('model', [1, 2, 3], [entry()])
        self.assertEqual(too_small.nbytes, 0)
        self.assertEqual(len(too_small), 0)

    def test_counts_only_completed_evaluation_and_preserves_all_arrays(self):
        arrays = [Array(3), Array(5), Array(2)]
        observed = []
        calls = []
        def evaluate(*batch):
            calls.extend(batch)
            for array in batch:
                array.done = True
        def report(done, total):
            self.assertEqual(done, sum(a.nbytes for a in arrays if a.done))
            self.assertEqual(total, 10)
            observed.append(done)
        evaluate_weights(evaluate, {"layer": arrays, "unused": None}, report, 8)
        self.assertEqual(calls, arrays)
        self.assertEqual(observed, [0, 8, 10])

    def test_failure_never_reports_full_completion(self):
        observed = []
        def evaluate(*batch):
            raise RuntimeError("allocation failed")
        with self.assertRaisesRegex(RuntimeError, "allocation failed"):
            evaluate_weights(evaluate, {"weights": Array(10)}, lambda d, t: observed.append(d))
        self.assertEqual(observed, [0])

    def test_empty_parameters_preserve_original_call(self):
        self.assertEqual(evaluate_weights(lambda x: x, {}, lambda d, t: self.fail()), {})

    def test_shared_parameters_are_counted_once(self):
        array = Array(10)
        calls = []
        totals = []
        evaluate_weights(lambda *batch: calls.extend(batch), {"a": array, "alias": array}, lambda d, t: totals.append(t))
        self.assertEqual(calls, [array])
        self.assertEqual(totals, [10, 10])

    @unittest.skipUnless(importlib.util.find_spec("mlx"), "native MLX checked on macOS release runner")
    def test_native_mlx_values_are_unchanged_after_measured_evaluation(self):
        import mlx.core as mx
        mx.set_default_device(mx.cpu)
        arrays = [mx.arange(32), mx.arange(16) * 3]
        totals = []
        evaluate_weights(mx.eval, {"layers": arrays}, lambda d, t: totals.append((d, t)), 64)
        self.assertEqual(arrays[0].tolist(), list(range(32)))
        self.assertEqual(arrays[1].tolist(), [n * 3 for n in range(16)])
        self.assertEqual(totals[-1], (sum(a.nbytes for a in arrays),) * 2)


if __name__ == "__main__":
    unittest.main()
