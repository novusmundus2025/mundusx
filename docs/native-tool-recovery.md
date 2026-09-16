# Native tool-call recovery

The contributor node holds tool-call deltas until the provider has finished and
every call in the batch has valid JSON object arguments, an ID, and a name.
Assistant text still streams immediately. Once validated, tool fragments retain
their original sizes and order when forwarded to the control plane.

An incomplete JSON argument or explicit generation-limit termination triggers
one new model request, using the original conversation and tool definitions plus
an instruction to use small targeted edits. Rejected arguments are discarded;
they are never executed or concatenated with the retry. Retry prose is suppressed
to avoid duplicating the already-streamed introduction. A failed retry is terminal.
Transport failures are not automatically replayed by this recovery path.

This is a contributor-node change. Updating Railway Chat or the Continue client
alone does not enable it. Roll out the rebuilt node on the machines serving native
tool requests, preserving their configuration and identity. Verify a fresh
Continue edit through the live endpoint before claiming production resolution.

Validation:

- All 131 node tests pass with `cargo test -p opengpu-node-agent --bin opengpu-node-agent -- --test-threads=1`.
- An HTTP fixture sends malformed streamed edit arguments followed by a valid
  replacement. The client receives only the valid edit, exactly once, and its
  arguments produce the expected temporary `menu.js` contents.
- Regression coverage includes a large incomplete string, an invalid second
  call in a batch, a valid whole-file edit that is replanned as a bounded patch,
  bounded retries, no retry on lost transport, preservation of tool definitions
  and token limits, and immediate ordinary-text streaming.

Failures report only safe diagnostics: tool name, argument character count, and
finish reason. Source text and partial arguments are never included in the error.

These tests validate recovery behavior, not an assurance that every model can
produce a correct patch. Tool-schema checks and project validation remain the
client's responsibility. No syntax completion or invented file contents are used.
