#!/usr/bin/env node

const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const HOME_CONFIG_DIR = path.join(os.homedir(), ".opengpu");
const LOCAL_CONFIG_DIR = path.join(process.cwd(), ".opengpu");
const CONFIG_DIR = process.env.OPENGPU_HOME || HOME_CONFIG_DIR;
const CONFIG_PATH = path.join(CONFIG_DIR, "config.json");
const LOCAL_CONFIG_PATH = path.join(LOCAL_CONFIG_DIR, "config.json");

const SAMPLE_NODES = [
  {
    nodeId: "m-001",
    backend: "m",
    status: "idle",
    availableMemoryMb: 24576,
    availableGpuPercent: 72,
    updatedAt: new Date().toISOString(),
    label: "MacBook M-series",
  },
  {
    nodeId: "cuda-001",
    backend: "cuda",
    status: "online",
    availableMemoryMb: 49152,
    availableGpuPercent: 84,
    updatedAt: new Date().toISOString(),
    label: "CUDA Worker",
  },
  {
    nodeId: "cuda-002",
    backend: "cuda",
    status: "offline",
    availableMemoryMb: 32768,
    availableGpuPercent: 0,
    updatedAt: new Date().toISOString(),
    label: "Dead CUDA Worker",
  },
];

function help() {
  process.stdout.write(`opengpu CLI

Commands:
  init
  login
  connect
  status
  contribute --m | --cuda
  pause
  resume
  logs
  update
`);
}

function ensureConfigDir() {
  try {
    fs.mkdirSync(CONFIG_DIR, { recursive: true });
  } catch (error) {
    if (CONFIG_DIR !== LOCAL_CONFIG_DIR) {
      fs.mkdirSync(LOCAL_CONFIG_DIR, { recursive: true });
      return;
    }

    throw error;
  }
}

function resolveConfigPath() {
  if (fs.existsSync(CONFIG_PATH)) {
    return CONFIG_PATH;
  }

  if (fs.existsSync(LOCAL_CONFIG_PATH)) {
    return LOCAL_CONFIG_PATH;
  }

  return CONFIG_PATH;
}

function readConfig() {
  const resolvedPath = resolveConfigPath();

  try {
    return JSON.parse(fs.readFileSync(resolvedPath, "utf8"));
  } catch {
    return null;
  }
}

function writeConfig(config) {
  try {
    ensureConfigDir();
    fs.writeFileSync(CONFIG_PATH, JSON.stringify(config, null, 2) + "\n");
    return CONFIG_PATH;
  } catch (error) {
    if (CONFIG_DIR === LOCAL_CONFIG_DIR) {
      throw error;
    }

    fs.mkdirSync(LOCAL_CONFIG_DIR, { recursive: true });
    fs.writeFileSync(LOCAL_CONFIG_PATH, JSON.stringify(config, null, 2) + "\n");
    return LOCAL_CONFIG_PATH;
  }
}

function makeDefaultConfig() {
  return {
    version: 1,
    deviceId: `node-${Math.random().toString(36).slice(2, 10)}`,
    backendPreference: null,
    connected: false,
    paused: false,
    controlPlaneUrl: "https://api.opengpu.ai",
  };
}

function parseArgs(argv) {
  const [command = "help", ...rest] = argv;
  const flags = new Set(rest);
  return { command, rest, flags };
}

function chooseNode(preference) {
  const liveNodes = SAMPLE_NODES.filter((node) => node.status !== "offline");
  const preferred = preference ? liveNodes.filter((node) => node.backend === preference) : liveNodes;
  const candidates = preferred.length > 0 ? preferred : liveNodes;

  return candidates.sort((a, b) => {
    const aScore = (a.status === "idle" ? 10 : 4) + a.availableMemoryMb / 1024 + a.availableGpuPercent / 10;
    const bScore = (b.status === "idle" ? 10 : 4) + b.availableMemoryMb / 1024 + b.availableGpuPercent / 10;
    return bScore - aScore;
  })[0] ?? null;
}

function printStatus(config) {
  const selected = chooseNode(config.backendPreference);
  const lines = [
    `deviceId: ${config.deviceId}`,
    `connected: ${config.connected ? "yes" : "no"}`,
    `paused: ${config.paused ? "yes" : "no"}`,
    `backendPreference: ${config.backendPreference ?? "auto"}`,
    `controlPlaneUrl: ${config.controlPlaneUrl}`,
    `bestLiveNode: ${selected ? `${selected.nodeId} (${selected.backend})` : "none"}`,
  ];

  process.stdout.write(lines.join("\n") + "\n");
}

function main() {
  const { command, flags } = parseArgs(process.argv.slice(2));
  const configPath = resolveConfigPath();
  const configExists = fs.existsSync(configPath);
  const config = configExists ? readConfig() : makeDefaultConfig();

  if (command === "help" || command === "--help" || command === "-h") {
    help();
    return;
  }

  if (command === "init") {
    if (configExists) {
      process.stdout.write(`config already exists at ${configPath}\n`);
      return;
    }

    const createdPath = writeConfig(config);
    process.stdout.write(`initialized ${createdPath}\n`);
    return;
  }

  if (!configExists) {
    process.stdout.write(`run "opengpu init" first\n`);
    process.exitCode = 1;
    return;
  }

  switch (command) {
    case "login": {
      config.connected = false;
      writeConfig(config);
      process.stdout.write(`login flow not wired yet, but config is ready at ${configPath}\n`);
      break;
    }
    case "connect": {
      config.connected = true;
      config.paused = false;
      writeConfig(config);
      process.stdout.write(`connected device ${config.deviceId}\n`);
      break;
    }
    case "status": {
      printStatus(config);
      break;
    }
    case "contribute": {
      if (flags.has("--m")) {
        config.backendPreference = "m";
      } else if (flags.has("--cuda")) {
        config.backendPreference = "cuda";
      } else {
        process.stdout.write(`choose a backend with --m or --cuda\n`);
        process.exitCode = 1;
        return;
      }

      writeConfig(config);
      process.stdout.write(`set backend preference to ${config.backendPreference}\n`);
      break;
    }
    case "pause": {
      config.paused = true;
      writeConfig(config);
      process.stdout.write(`paused contribution\n`);
      break;
    }
    case "resume": {
      config.paused = false;
      writeConfig(config);
      process.stdout.write(`resumed contribution\n`);
      break;
    }
    case "logs": {
      process.stdout.write(`no logs yet; node agent and control plane are still stubs\n`);
      break;
    }
    case "update": {
      process.stdout.write(`update channel not wired yet\n`);
      break;
    }
    default: {
      process.stdout.write(`opengpu: unknown command "${command}"\n`);
      process.exitCode = 1;
    }
  }
}

main();
