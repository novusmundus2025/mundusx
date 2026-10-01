"""Exercise a built CLI against a local mock; never downloads real models."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

loaded = True
requests = []


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def reply(self, body, status=200):
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(json.dumps(body).encode())

    def do_GET(self):
        if self.path == "/api/v1/models":
            self.reply({"models": [{"key": "file", "type": "llm", "loaded_instances":
                [{"id": "chosen", "config": {"context_length": 4096}}] if loaded else []}]})
        elif self.path == "/v1/models":
            self.reply({"data": [{"id": "chosen", "owned_by": "lmstudio"}]})
        else:
            self.reply({"error": "not found"}, 404)

    def do_POST(self):
        assert self.path == "/v1/chat/completions", self.path
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        requests.append(body)
        self.reply({"choices": [{"message": {"role": "assistant", "content": "OK"}}]})


binary = str(Path(sys.argv[1]).resolve())
server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
threading.Thread(target=server.serve_forever, daemon=True).start()
url = f"http://127.0.0.1:{server.server_port}"
try:
    with tempfile.TemporaryDirectory(prefix="mundusx-connection-test-") as root:
        env = dict(os.environ, OPENGPU_HOME=root, NO_COLOR="1")

        def run(*args, code=0):
            result = subprocess.run([binary, *args], env=env, capture_output=True, text=True, timeout=30)
            assert result.returncode == code, (args, result.returncode, result.stdout, result.stderr)
            return result.stdout + result.stderr

        scan = json.loads(run("cluster", "scan", "--url", url, "--json"))
        assert scan["detected"][0]["kind"] == "lm-studio"
        assert requests == [], "Scanning must not load or generate from a model"
        run("cluster", "use", url + "/v1", "--model", "chosen")
        config_path = Path(root, "config.json")
        original = config_path.read_bytes()
        cluster = json.loads(original)["contributed_cluster"]
        assert cluster["model"] == "chosen" and cluster["models"] == ["chosen"]
        assert cluster["base_url"] == url
        assert requests and all(r["model"] == "chosen" for r in requests)
        loaded = False
        run("cluster", "use", url, "--model", "chosen", code=1)
        assert config_path.read_bytes() == original
        loaded = True
        result = run("install", "--connection", "pair", "--cluster-url", url,
                     "--cluster-model", "chosen", code=2)
        assert "PAIR route verified" in result
        assert config_path.read_bytes() == original, "PAIR validation must not mutate configuration"
        run("install", "--connection", "managed", "--cluster-url", url, code=2)
        run("install", "--connection", "direct", "--no-contribute-cluster", code=2)
        assert config_path.read_bytes() == original
        print("PASS: discovery, exact model, unload, URL normalization, PAIR gate, and conflicting options")
finally:
    server.shutdown()
