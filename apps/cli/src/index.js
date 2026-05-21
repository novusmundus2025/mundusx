#!/usr/bin/env node

const args = process.argv.slice(2);
const [command = "help"] = args;

const help = `opengpu CLI

Commands:
  init
  login
  connect
  status
  contribute --m
  contribute --cuda
  pause
  resume
  logs
  update
`;

if (command === "help" || command === "--help" || command === "-h") {
  process.stdout.write(help);
  process.exit(0);
}

process.stdout.write(`opengpu: command not implemented yet -> ${args.join(" ")}\n`);

