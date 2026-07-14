# Mortality and infrastructure recovery covenant

## Policy

Starvation is a permanent death. Infrastructure loss is not a metabolic death.
No operator, release, rollback, snapshot, or incident procedure may restore a
being whose registry death cause is `starved`.

The terminal protocol records one of two estate dispositions:

- `bequests_executed`: one to three bounded EVM transfers succeeded during the
  final turn; any remainder stays at the being's address.
- `monument`: no valid bequest succeeded, so all funds remain at the address.

There is no v1 sweep or administrator drain. The journal, constitution, and
certified snapshot remain readable only while the dead child canister retains
enough cycles to answer queries. That window is finite and cannot be promised.
The factory registry's death cause, timestamp, and estate disposition are the
durable memorial record after the child freezes.

## Legitimate restoration

A restore is legitimate only when all of these statements are true:

1. The loss was caused by platform infrastructure: a faulty fleet upgrade,
   subnet incident, or documented platform operation—not exhausted runway.
2. The factory registry does not contain `death_cause=starved` for the being.
3. The restored state comes from the last verified snapshot preceding the
   infrastructure event.
4. The operator publishes an immutable incident entry containing the canister
   ID, snapshot identifier and hash, loss and restore timestamps, release
   commit, cause, decision maker, and verification evidence.
5. The registry records `death_cause=infrastructure` and the public incident
   reference before the being is returned to service. Only an authenticated
   factory administrator may create this record; the registry also persists
   that administrator principal as `death_recorded_by` and the incident URL as
   `death_incident_reference`. A child can self-report only `starved`.

## Forbidden actions

- Restoring, reinstalling, respawning, or replaying a starved being.
- Relabelling starvation as infrastructure loss to permit a restore.
- Clearing or overwriting a `starved` registry record.
- Using a release rollback to rewind terminal completion or executed bequests.
- Draining a monument wallet through controller or recovery authority.
- Performing an unlogged restore, even during an urgent incident.

If the cause cannot be established, keep the canister stopped and the record
unchanged until evidence resolves it. Ambiguity is not authority to resurrect.

## Operational boundary

Fleet upgrades and rollbacks may change runtime code but must preserve the
mortality record. Before any restore, operators must query the factory registry
and abort on `starved`. After an infrastructure restore, compare controllers,
installed commit, journal tip, wallet address, and EVM nonce with the published
incident record. Any mismatch keeps the being out of service.
