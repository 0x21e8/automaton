import { createHash } from "node:crypto";

import { validateReleaseManifest } from "./release-manifest.mjs";

const SOURCE_COMMIT = /^[0-9a-f]{40}$/;

export function renderReleaseManifest({
  sourceCommit,
  dirty,
  mode,
  environmentVersion,
  imageRefs,
  artifacts,
  createdAt
}) {
  if (!SOURCE_COMMIT.test(sourceCommit)) {
    throw new Error("sourceCommit must be a lowercase 40-character SHA");
  }
  if (mode !== "dry-run" && mode !== "publish") {
    throw new Error(`mode must be dry-run or publish, got ${mode}`);
  }
  if (typeof dirty !== "boolean") throw new Error("dirty must be a boolean");
  if (mode === "publish" && dirty) throw new Error("dirty releases are not publishable");

  const manifest = {
    schemaVersion: 2,
    release: {
      sourceCommit,
      environmentVersion,
      createdAt,
      ...(dirty ? { dirty: true } : {})
    },
    images: {
      web: image(imageRefs?.web, "web", sourceCommit),
      indexer: image(imageRefs?.indexer, "indexer", sourceCommit),
      rpcGateway: image(imageRefs?.rpcGateway, "rpcGateway", sourceCommit)
    },
    artifacts: {
      automatonWasm: artifact(artifacts?.automatonWasm, "automatonWasm", sourceCommit),
      factoryWasm: artifact(artifacts?.factoryWasm, "factoryWasm", sourceCommit)
    },
    ops: { sourceCommit }
  };

  return validateReleaseManifest(manifest, { allowDirty: mode === "dry-run" });
}

function artifact(value, name, sourceCommit) {
  if (!value || typeof value.fileName !== "string" || !Buffer.isBuffer(value.bytes)) {
    throw new Error(`${name} artifact requires a fileName and byte buffer`);
  }
  return {
    fileName: value.fileName,
    sha256: createHash("sha256").update(value.bytes).digest("hex"),
    sourceCommit
  };
}

function image(ref, name, sourceCommit) {
  if (typeof ref !== "string") throw new Error(`${name} image ref must be an immutable digest ref`);
  const at = ref.lastIndexOf("@");
  if (at <= 0) throw new Error(`${name} image ref must be an immutable digest ref`);
  return {
    ref,
    digest: ref.slice(at + 1),
    repository: ref.slice(0, at),
    tag: sourceCommit
  };
}
