import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const defaultRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const rootOption = process.argv.indexOf("--root");
const root = path.resolve(rootOption === -1 ? defaultRoot : process.argv[rootOption + 1]);
const write = process.argv.includes("--write");
const check = process.argv.includes("--check");
if (write === check) throw new Error("use exactly one of --write or --check");

const source = JSON.parse(fs.readFileSync(path.join(root, "packages/ui/tokens.json"), "utf8"));
const terminalSource = fs.readFileSync(path.join(root, "packages/shared/src/terminal-commands.ts"), "utf8");
const cssHeader = "/* GENERATED FILE - DO NOT HAND-EDIT. Run npm run generate:ui-assets. */\n";
const tsHeader = "// GENERATED FILE - DO NOT HAND-EDIT. Run npm run generate:ui-assets.\n";
const outputs = new Map([
  ["packages/ui/src/generated/tokens.ts", `${tsHeader}\nexport const uiTokens = ${JSON.stringify(source, null, 2)} as const;\n\nexport type UiTheme = keyof typeof uiTokens.themes;\n`],
  ["packages/ui/src/generated/themes.css", `${cssHeader}\n${renderCss(source)}\n`],
  ["components/ic-automaton/src/ui_tokens.css", `${cssHeader}\n${renderCss(source, "direct-console")}\n`],
  ["components/ic-automaton/src/ui_terminal_commands.js", renderTerminalCommands(terminalSource)]
]);

if (check) {
  const stale = [...outputs].filter(([relativePath, expected]) => {
    const filePath = path.join(root, relativePath);
    return !fs.existsSync(filePath) || fs.readFileSync(filePath, "utf8") !== expected;
  }).map(([relativePath]) => relativePath);
  if (stale.length > 0) throw new Error(`generated UI assets are stale: ${stale.join(", ")}`);
  console.log("UI token assets are current.");
} else {
  for (const [relativePath, content] of outputs) {
    const filePath = path.join(root, relativePath);
    fs.mkdirSync(path.dirname(filePath), { recursive: true });
    fs.writeFileSync(filePath, content);
  }
  console.log(`wrote ${outputs.size} UI token assets.`);
}

function renderCss(tokens, onlyTheme = null) {
  const lines = [":root {"];
  for (const [group, values] of Object.entries(tokens)) {
    if (["brand", "themes"].includes(group)) continue;
    for (const [key, value] of Object.entries(values)) lines.push(`  --ui-${toKebab(key)}: ${value};`);
  }
  lines.push("}");
  const themes = onlyTheme === null ? Object.entries(tokens.themes) : [[onlyTheme, tokens.themes[onlyTheme]]];
  for (const [name, values] of themes) {
    const selector = onlyTheme === null ? `[data-ui-theme=\"${name}\"]` : ":root";
    lines.push(`${selector} {`);
    for (const [key, value] of Object.entries(values)) lines.push(`  --ui-${toKebab(key)}: ${value};`);
    lines.push("}");
  }
  return lines.join("\n");
}

function toKebab(value) {
  return value.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`).replace(/_/g, "-");
}

function renderTerminalCommands(sourceText) {
  const rows = [...sourceText.matchAll(/\["([^"\\]+)",\s*"((?:[^"\\]|\\.)*)",\s*"((?:[^"\\]|\\.)*)",\s*"([^"\\]+)",\s*"([^"\\]+)",\s*"([^"\\]+)"\]/g)]
    .map((match) => ({
      name: match[1], usage: JSON.parse(`"${match[2]}"`), summary: JSON.parse(`"${match[3]}"`),
      authLevel: match[4], transport: match[5], mode: match[6]
    }));
  if (rows.length === 0) throw new Error("no terminal commands found in canonical metadata");
  return `// GENERATED FILE - DO NOT HAND-EDIT. Run npm run generate:ui-assets.\nexport const CANONICAL_TERMINAL_COMMANDS = ${JSON.stringify(rows, null, 2)};\n`;
}
