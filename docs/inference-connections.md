# Inference connections

Available in release 0.2.20 and newer. Use matching CLI, agent, and installer
versions. The older 0.2.19 bundle does not include these changes.

`opengpu install` offers one connection mode. Arrow keys move; Space or Enter
chooses a mode. Workload checkboxes still use Space to toggle and Enter to continue.

## MundusX-managed models

Choose **MundusX-managed runtime**, or run:

```sh
opengpu install --connection managed
```

Download or reuse a standalone model through the existing model picker. Changing
connections does not delete downloaded models. Only the active runtime needs RAM.

## Direct local engine

Start your Ollama, LM Studio, vLLM, or llama.cpp server first. In LM Studio, load
the selected LLM and enable the API server. Both its native model API and its
OpenAI-compatible inference API must be reachable without authentication in this
initial integration; authenticated endpoints are not yet supported.

```sh
opengpu install --connection direct --cluster-url http://127.0.0.1:1234
```

Choose an exact model from the list. For noninteractive setup with multiple models:

```sh
opengpu install --connection direct --cluster-url http://127.0.0.1:1234/v1 --cluster-model YOUR_MODEL_ID
```

The optional `/v1` suffix is normalized. Setup tests a short completion, then
stores the selected model. Scanning endpoints does not generate completions or
load models. LM Studio's native v1 API (v0 fallback) supplies loaded model IDs;
downloaded but unloaded models are not offered as ready.

```sh
opengpu start --background
opengpu status
opengpu doctor
```

The external service remains under your control. The agent checks the selected
model before claiming and executing jobs. If it disappears, new claims stop;
they resume when the same model becomes ready again. Requests for other models
are refused. There is still a race if a model is unloaded after a check: that
request can fail. An in-flight request is not automatically rerouted.

After reboot, start the external service and load the model, then start OpenGPU.
A reachable model listing alone does not prove a model is warm. Existing external
engine memory settings remain owned by that engine; MundusX does not change them.

## NVIDIA PAIR: validate a route only

Copy an endpoint from PAIR's **Endpoints** screen:

```sh
opengpu install --connection pair --cluster-url http://127.0.0.1:11434 --cluster-model YOUR_MODEL_ID
```

This probes the route and runs a short inference test, then exits with code 2 and
an explanation that contribution is not enabled. It does not save a contributor
connection, change existing configuration, or start contribution.

PAIR can route a localhost request to another computer. Serving-node limits and
duplicate capacity registration are not implemented, so one job per cluster is
not sufficient to enable public contribution safely. PAIR identity is selected
explicitly here; transparent proxies cannot reliably be identified by port or
OpenAI-compatible model responses. Do not label a PAIR proxy as a direct engine.
For separate contributors, use each machine's actual engine endpoint, not PAIR's
shared endpoint. Automatic PAIR discovery and duplicate-cluster detection remain
future work.

## Bootstrap scripts

Linux/macOS bootstrap keeps installation separate from interactive setup:

```sh
bash install.sh --connection direct --cluster-url http://127.0.0.1:1234
```

It skips automatic managed vLLM provisioning for direct/PAIR mode and prints the
matching setup command. With `--auto-start`, it forwards connection/model options
to setup. A failed verification or PAIR gate stops the script before startup.

Windows:

```powershell
.\install.ps1 -Connection direct -ClusterUrl http://127.0.0.1:1234
```

This skips bundled managed GPU runtime downloads and passes the connection options
to the guided CLI window. `-ClusterModel YOUR_MODEL_ID` selects an exact model.
Use `-Connection managed` for standalone models. These options cannot be combined
with forced GPU runtime installation when using an external connection.
