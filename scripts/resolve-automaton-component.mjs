import { execFileSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const rootDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const resolver = path.join(rootDir, "scripts", "resolve-automaton-component.sh");

export function resolveAutomatonComponentRoot(env = process.env) {
  return execFileSync("sh", [resolver], {
    cwd: rootDir,
    env: { ...env, AUTOMATON_LAUNCHPAD_ROOT: rootDir },
    encoding: "utf8"
  }).trim();
}

if (import.meta.url === `file://${process.argv[1]}`) {
  process.stdout.write(`${resolveAutomatonComponentRoot()}\n`);
}
