import fs from "node:fs";
import path from "node:path";

const root = process.cwd();
const dist = path.join(root, "apps/web/dist");
if (!fs.existsSync(dist)) throw new Error(`missing public web build: ${dist}`);

const files = walk(dist).filter((file) => /\.(?:js|css|html)$/.test(file));
const forbidden = ["apps/evaluator-web", "Operator / Evaluation", "stopCurrentRun", "subscribeToEvaluatorEvents"];
const matches = [];
for (const file of files) {
  const body = fs.readFileSync(file, "utf8");
  for (const marker of forbidden) if (body.includes(marker)) matches.push(`${file}: ${marker}`);
}
if (matches.length > 0) throw new Error(`public bundle imports evaluator surface:\n${matches.join("\n")}`);
console.log(`public bundle boundary passed (${files.length} assets checked)`);

function walk(directory) {
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const absolute = path.join(directory, entry.name);
    return entry.isDirectory() ? walk(absolute) : [absolute];
  });
}
