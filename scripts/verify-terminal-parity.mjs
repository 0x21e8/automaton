import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const canonicalPath = path.join(root, "packages/shared/src/terminal-commands.ts");
const generatedPath = path.join(root, "components/ic-automaton/src/ui_terminal_commands.js");
const embeddedPath = path.join(root, "components/ic-automaton/src/ui_app.js");
const registryPath = path.join(root, "apps/web/src/lib/cli-command-registry.ts");

export function verifyTerminalParity({ canonicalSource, generatedSource, embeddedSource, registrySource }) {
  const canonical = extractCanonicalNames(canonicalSource);
  const generated = extractGeneratedNames(generatedSource);
  const dispatcher = embeddedSource.match(/switch \(cmd\) \{([\s\S]*?)\s*default:/)?.[1] ?? "";
  const embedded = [...dispatcher.matchAll(/case\s+["']([^"']+)["']\s*:/g)].map((match) => match[1]);
  const missing = (left, right) => left.filter((name) => !right.includes(name));
  const failures = [
    ...missing(canonical, generated).map((name) => `generated metadata missing ${name}`),
    ...missing(generated, canonical).map((name) => `generated metadata has unknown ${name}`),
    ...missing(canonical, embedded).map((name) => `embedded dispatcher missing ${name}`),
    ...missing(embedded, canonical).map((name) => `embedded dispatcher has unknown ${name}`)
  ];
  if (!registrySource.includes("terminalCommandRegistry") || !registrySource.includes("@ic-automaton/shared")) {
    failures.push("Lab command registry is not sourced from @ic-automaton/shared");
  }
  if (failures.length > 0) throw new Error(`terminal parity failed:\n- ${failures.join("\n- ")}`);
  return { commandCount: canonical.length };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const result = verifyTerminalParity({
    canonicalSource: fs.readFileSync(canonicalPath, "utf8"),
    generatedSource: fs.readFileSync(generatedPath, "utf8"),
    embeddedSource: fs.readFileSync(embeddedPath, "utf8"),
    registrySource: fs.readFileSync(registryPath, "utf8")
  });
  console.log(`terminal parity passed (${result.commandCount} canonical commands)`);
}

function extractCanonicalNames(source) {
  return [...source.matchAll(/\["([^"\\]+)",\s*"/g)].map((match) => match[1]);
}

function extractGeneratedNames(source) {
  return [...source.matchAll(/"name":\s*"([^"]+)"/g)].map((match) => match[1]);
}
