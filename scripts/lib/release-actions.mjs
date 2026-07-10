export const RELEASE_ACTIONS = Object.freeze({
  soft: Object.freeze(["pull-images", "compose-up", "health", "smoke", "record"]),
  "hard-reset": Object.freeze(["pull-images", "compose-up", "reset", "bootstrap", "record"]),
  "admit-child": Object.freeze(["upload-child", "verify-factory-health", "record"]),
  "upgrade-named": Object.freeze(["snapshot", "upgrade", "verify", "record"])
});

export function assertAllowedOperations(mode, operations) {
  const allowed = RELEASE_ACTIONS[mode];
  if (!allowed) throw new Error(`unknown release mode: ${mode}`);
  const forbidden = operations.filter((operation) => !allowed.includes(operation));
  if (forbidden.length > 0) {
    throw new Error(`${mode} action invoked forbidden operations: ${forbidden.join(", ")}`);
  }
}
