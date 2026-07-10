import fs from "node:fs";

export const RELEASE_SCHEMA_VERSION = 2;
const SHA1 = /^[0-9a-f]{40}$/;
const SHA256 = /^[a-f0-9]{64}$/;
const IMAGE_DIGEST = /^sha256:[a-f0-9]{64}$/;

export function readReleaseManifest(filePath, options = {}) {
  return validateReleaseManifest(JSON.parse(fs.readFileSync(filePath, "utf8")), options);
}

export function validateReleaseManifest(value, { allowDirty = false } = {}) {
  const manifest = object(value, "manifest");
  if (manifest.schemaVersion !== RELEASE_SCHEMA_VERSION) fail("schemaVersion must be 2");
  const release = object(manifest.release, "release");
  if ("mode" in release || "action" in release) {
    fail("release must not contain deployment mode or action");
  }
  const sourceCommit = sha1(release.sourceCommit, "release.sourceCommit");
  const environmentVersion = string(release.environmentVersion, "release.environmentVersion");
  const createdAt = string(release.createdAt, "release.createdAt");
  if (Number.isNaN(Date.parse(createdAt))) fail("release.createdAt must be ISO-8601");
  if (release.dirty === true && !allowDirty) fail("dirty releases are not publishable");
  if (release.dirty !== undefined && typeof release.dirty !== "boolean") fail("release.dirty must be boolean");
  const overrides = Array.isArray(release.reviewedSourceOverrides)
    ? release.reviewedSourceOverrides.map((entry, index) => string(entry, `release.reviewedSourceOverrides[${index}]`))
    : undefined;

  const images = object(manifest.images, "images");
  const artifacts = object(manifest.artifacts, "artifacts");
  const normalized = {
    schemaVersion: RELEASE_SCHEMA_VERSION,
    release: {
      sourceCommit,
      environmentVersion,
      createdAt,
      ...(release.dirty === true ? { dirty: true } : {}),
      ...(overrides ? { reviewedSourceOverrides: overrides } : {})
    },
    images: {
      web: image(images.web, "images.web"),
      indexer: image(images.indexer, "images.indexer"),
      rpcGateway: image(images.rpcGateway, "images.rpcGateway")
    },
    artifacts: {
      automatonWasm: artifact(artifacts.automatonWasm, "artifacts.automatonWasm"),
      factoryWasm: artifact(artifacts.factoryWasm, "artifacts.factoryWasm")
    },
    ops: { sourceCommit: sha1(object(manifest.ops, "ops").sourceCommit, "ops.sourceCommit") }
  };
  if (normalized.ops.sourceCommit !== sourceCommit) fail("ops.sourceCommit must equal release.sourceCommit");
  for (const [name, value] of Object.entries(normalized.artifacts)) {
    if (value.sourceCommit !== sourceCommit && !overrides?.includes(`artifacts.${name}`)) {
      fail(`${name}.sourceCommit differs without a reviewed source override`);
    }
  }
  return normalized;
}

function object(value, label) {
  if (typeof value !== "object" || value === null || Array.isArray(value)) fail(`${label} must be an object`);
  return value;
}

function string(value, label) {
  if (typeof value !== "string" || value.trim() === "") fail(`${label} must be a non-empty string`);
  return value.trim();
}

function sha1(value, label) {
  const normalized = string(value, label);
  if (!SHA1.test(normalized)) fail(`${label} must be lowercase 40-character hex`);
  return normalized;
}

function image(value, label) {
  const entry = object(value, label);
  const ref = string(entry.ref, `${label}.ref`);
  const digest = string(entry.digest, `${label}.digest`);
  if (!IMAGE_DIGEST.test(digest)) fail(`${label}.digest must be sha256 hex`);
  const at = ref.lastIndexOf("@");
  if (at <= 0 || ref.slice(at + 1) !== digest) fail(`${label}.ref must equal repository@digest`);
  const repository = entry.repository === undefined ? undefined : string(entry.repository, `${label}.repository`);
  const tag = entry.tag === undefined ? undefined : string(entry.tag, `${label}.tag`);
  if (repository && ref !== `${repository}@${digest}`) fail(`${label}.ref repository mismatch`);
  return { ref, digest, ...(repository ? { repository } : {}), ...(tag ? { tag } : {}) };
}

function artifact(value, label) {
  const entry = object(value, label);
  const fileName = string(entry.fileName, `${label}.fileName`);
  if (fileName !== fileName.split(/[\\/]/).pop() || !/^[-A-Za-z0-9_.]+$/.test(fileName)) {
    fail(`${label}.fileName must be a safe file name`);
  }
  const sha256 = string(entry.sha256, `${label}.sha256`);
  if (!SHA256.test(sha256)) fail(`${label}.sha256 must be lowercase 64-character hex`);
  return { fileName, sha256, sourceCommit: sha1(entry.sourceCommit, `${label}.sourceCommit`) };
}

function fail(message) {
  throw new Error(`invalid release manifest: ${message}`);
}
