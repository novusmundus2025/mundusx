# Media-only contribution

Deploy the companion mundusx/control-plane change first. Update both the CLI and node agent.

```bash
opengpu install --private --control-plane-url https://YOUR_CONTROL_PLANE --cap-percent 80 --workloads image,video,image-to-video --no-contribute-cluster
opengpu media setup --yes
opengpu media --video setup --yes
opengpu media --image-to-video --fast setup --yes
opengpu media verify
opengpu media --video --seconds 10 verify
opengpu media --image-to-video --fast --seconds 10 verify --input-image /home/a/.opengpu/media/artifacts/r.png
export MUNDUSX_MEDIA_SERVER_URL=https://YOUR_CHAT_SERVER
opengpu start
opengpu capabilities
```

Verification creates actual media. Only exact verified durations, profiles, caps and endpoints are advertised. Verify other durations separately. The private server URL names the chat service associated with your plane.

Media-only nodes retain signed registration, operator admission, power/memory checks and one media slot. They cannot claim LLM jobs or warm a vLLM/llama runtime. If model policy is enforced, operators must allow the selected Qwen/Wan media model identities.

Fast I2V uses Wan2.2 I2V A14B FP8, matching high/low Lightning LoRAs, 832x480, four steps (2+2), CFG 1, Euler/simple, 16 fps and aspect-preserving black padding. Reference images arrive through signed, lease-authorized requests, are checksum checked, and are removed locally after generation.

No novusmundus2025 mirror update, production deployment, or GX10 generation is included in this source change.
