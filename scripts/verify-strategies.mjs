import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const rootDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const strategiesDir = path.join(rootDir, "strategies");
const manifestPath = path.join(strategiesDir, "manifest.json");
const factorySourcePath = path.join(rootDir, "backend/factory/src/strategy_repository.rs");
const legacySeedDirectory = ["strategy", "seeds"].join("-");

const manifest = readJson(manifestPath);
const entries = manifest.strategies;
const seenIds = new Set();
const recipePaths = new Set();

if (manifest.format !== "launchpad.strategy-manifest.v2") {
  fail(`unsupported strategy manifest format: ${manifest.format}`);
}

for (const entry of entries) {
  if (seenIds.has(entry.strategy_id)) fail(`duplicate strategy ID: ${entry.strategy_id}`);
  seenIds.add(entry.strategy_id);

  const recipePath = path.join(strategiesDir, entry.recipe_file);
  const metadataPath = path.join(strategiesDir, path.dirname(entry.recipe_file), "metadata.json");
  recipePaths.add(path.relative(strategiesDir, recipePath));
  const recipe = readJson(recipePath);
  const metadata = readJson(metadataPath);

  for (const field of ["strategy_id", "protocol", "primitive", "canonical_chain_id"]) {
    const recipeField =
      field === "strategy_id"
        ? "template_id"
        : field === "canonical_chain_id"
          ? "chain_id"
          : field;
    if (recipe[recipeField] !== entry[field]) {
      fail(`${entry.strategy_id}: recipe ${recipeField} does not match manifest ${field}`);
    }
    if (metadata[field] !== entry[field]) {
      fail(`${entry.strategy_id}: metadata ${field} does not match manifest`);
    }
  }
  if (metadata.canonical_chain !== entry.canonical_chain) {
    fail(`${entry.strategy_id}: metadata canonical_chain does not match manifest`);
  }

  const canonicalJson = JSON.stringify(recipe);
  const digest = crypto.createHash("sha256").update(canonicalJson).digest("hex");
  if (digest !== entry.recipe_sha256) {
    fail(`${entry.strategy_id}: recipe_sha256 mismatch (expected ${entry.recipe_sha256}, got ${digest})`);
  }

  const factorySource = fs.readFileSync(factorySourcePath, "utf8");
  if (!factorySource.includes(`strategies/${entry.recipe_file}`)) {
    fail(`${entry.strategy_id}: factory does not embed its canonical recipe`);
  }
}

const recipeFiles = collectRecipeFiles(strategiesDir);
if (recipeFiles.length !== recipePaths.size || recipeFiles.some((file) => !recipePaths.has(file))) {
  fail("strategies/ contains a recipe that is missing from the canonical manifest");
}

const factorySource = fs.readFileSync(factorySourcePath, "utf8");
if (factorySource.includes(legacySeedDirectory)) {
  fail(`factory still references the removed ${legacySeedDirectory} directory`);
}

process.stdout.write(`strategy verification passed (${entries.length} canonical recipes)\n`);

function readJson(filePath) {
  if (!fs.existsSync(filePath)) fail(`missing strategy asset: ${path.relative(rootDir, filePath)}`);
  try {
    return JSON.parse(fs.readFileSync(filePath, "utf8"));
  } catch (error) {
    fail(`invalid JSON in ${path.relative(rootDir, filePath)}: ${error.message}`);
  }
}

function collectRecipeFiles(directory) {
  return fs
    .readdirSync(directory, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .flatMap((entry) => {
      const relative = path.join(entry.name, "recipe.json");
      return fs.existsSync(path.join(directory, relative)) ? [relative] : [];
    });
}

function fail(message) {
  throw new Error(`strategy verification failed: ${message}`);
}
