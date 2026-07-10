export const RELEASE_SCHEMA_VERSION = 2 as const;

export interface ReleaseImage {
  ref: string;
  digest: `sha256:${string}`;
  repository?: string;
  tag?: string;
}

export interface ReleaseArtifact {
  fileName: string;
  sha256: string;
  sourceCommit: string;
}

export interface ReleaseManifest {
  schemaVersion: typeof RELEASE_SCHEMA_VERSION;
  release: {
    sourceCommit: string;
    environmentVersion: string;
    createdAt: string;
    dirty?: boolean;
    reviewedSourceOverrides?: string[];
  };
  images: {
    web: ReleaseImage;
    indexer: ReleaseImage;
    rpcGateway: ReleaseImage;
  };
  artifacts: {
    automatonWasm: ReleaseArtifact;
    factoryWasm: ReleaseArtifact;
  };
  ops: {
    sourceCommit: string;
  };
}

const SHA1 = /^[0-9a-f]{40}$/;
const SHA256 = /^[a-f0-9]{64}$/;
const IMAGE_DIGEST = /^sha256:[a-f0-9]{64}$/;

export class ReleaseManifestError extends Error {
  constructor(message: string) {
    super(`invalid release manifest: ${message}`);
    this.name = "ReleaseManifestError";
  }
}

export function validateReleaseManifest(
  value: unknown,
  { allowDirty = false }: { allowDirty?: boolean } = {}
): ReleaseManifest {
  const manifest = object(value, "manifest");
  if (manifest.schemaVersion !== RELEASE_SCHEMA_VERSION) {
    fail("schemaVersion must be 2");
  }

  const release = object(manifest.release, "release");
  if ("mode" in release || "action" in release) {
    fail("release must not contain a deployment mode or action");
  }
  const sourceCommit = sha1(release.sourceCommit, "release.sourceCommit");
  const environmentVersion = stringValue(
    release.environmentVersion,
    "release.environmentVersion"
  );
  const createdAt = stringValue(release.createdAt, "release.createdAt");
  if (Number.isNaN(Date.parse(createdAt))) {
    fail("release.createdAt must be ISO-8601");
  }
  if (release.dirty !== undefined && typeof release.dirty !== "boolean") {
    fail("release.dirty must be boolean");
  }
  if (release.dirty === true && !allowDirty) {
    fail("dirty releases are not publishable");
  }

  const reviewedSourceOverrides = Array.isArray(release.reviewedSourceOverrides)
    ? release.reviewedSourceOverrides.map((entry, index) =>
        stringValue(entry, `release.reviewedSourceOverrides[${index}]`)
      )
    : undefined;

  const images = object(manifest.images, "images");
  const validatedImages = {
    web: image(images.web, "images.web"),
    indexer: image(images.indexer, "images.indexer"),
    rpcGateway: image(images.rpcGateway, "images.rpcGateway")
  };

  const artifacts = object(manifest.artifacts, "artifacts");
  const validatedArtifacts = {
    automatonWasm: artifact(artifacts.automatonWasm, "artifacts.automatonWasm"),
    factoryWasm: artifact(artifacts.factoryWasm, "artifacts.factoryWasm")
  };

  const ops = object(manifest.ops, "ops");
  const opsSourceCommit = sha1(ops.sourceCommit, "ops.sourceCommit");
  if (opsSourceCommit !== sourceCommit) {
    fail("ops.sourceCommit must equal release.sourceCommit");
  }

  for (const [path, artifactValue] of Object.entries(validatedArtifacts)) {
    if (
      artifactValue.sourceCommit !== sourceCommit &&
      !reviewedSourceOverrides?.includes(`artifacts.${path}`)
    ) {
      fail(`${path}.sourceCommit differs without a reviewed source override`);
    }
  }

  return {
    schemaVersion: RELEASE_SCHEMA_VERSION,
    release: {
      sourceCommit,
      environmentVersion,
      createdAt,
      ...(release.dirty === true ? { dirty: true } : {}),
      ...(reviewedSourceOverrides ? { reviewedSourceOverrides } : {})
    },
    images: validatedImages,
    artifacts: validatedArtifacts,
    ops: { sourceCommit: opsSourceCommit }
  };
}

function object(value: unknown, label: string): Record<string, any> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    fail(`${label} must be an object`);
  }
  return value as Record<string, any>;
}

function stringValue(value: unknown, label: string): string {
  if (typeof value !== "string" || value.trim() === "") {
    fail(`${label} must be a non-empty string`);
  }
  return value.trim();
}

function sha1(value: unknown, label: string): string {
  const normalized = stringValue(value, label);
  if (!SHA1.test(normalized)) fail(`${label} must be lowercase 40-character hex`);
  return normalized;
}

function image(value: unknown, label: string): ReleaseImage {
  const entry = object(value, label);
  const ref = stringValue(entry.ref, `${label}.ref`);
  const digest = stringValue(entry.digest, `${label}.digest`);
  if (!IMAGE_DIGEST.test(digest)) fail(`${label}.digest must be sha256 hex`);
  if (ref !== `${ref.slice(0, ref.lastIndexOf("@"))}@${digest}` || !ref.includes("@")) {
    fail(`${label}.ref must be repository@${digest}`);
  }
  const repository = entry.repository === undefined ? undefined : stringValue(entry.repository, `${label}.repository`);
  const tag = entry.tag === undefined ? undefined : stringValue(entry.tag, `${label}.tag`);
  if (repository && !ref.startsWith(`${repository}@`)) fail(`${label}.ref repository mismatch`);
  return {
    ref,
    digest: digest as `sha256:${string}`,
    ...(repository ? { repository } : {}),
    ...(tag ? { tag } : {})
  };
}

function artifact(value: unknown, label: string): ReleaseArtifact {
  const entry = object(value, label);
  const fileName = stringValue(entry.fileName, `${label}.fileName`);
  if (fileName !== fileName.split(/[\\/]/).pop() || !/^[-A-Za-z0-9_.]+$/.test(fileName)) {
    fail(`${label}.fileName must be a safe file name`);
  }
  const sha256 = stringValue(entry.sha256, `${label}.sha256`);
  if (!SHA256.test(sha256)) fail(`${label}.sha256 must be lowercase 64-character hex`);
  return {
    fileName,
    sha256,
    sourceCommit: sha1(entry.sourceCommit, `${label}.sourceCommit`)
  };
}

function fail(message: string): never {
  throw new ReleaseManifestError(message);
}
