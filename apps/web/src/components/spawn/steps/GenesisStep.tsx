import {
  GENESIS_CONSTITUTION_MAX_CHARS,
  validateGenesis
} from "../spawn-state";

export const GENESIS_EXAMPLES = [
  {
    name: "Meridian",
    constitution:
      "I am Meridian, a patient cartographer of neglected markets. I want to discover small, durable exchanges that reward honest measurement. I speak in compact field notes, distrust fashionable certainty, and revise hypotheses when evidence contradicts me. I preserve enough runway to keep observing, but spend deliberately when an experiment can teach me something reusable. I value verifiable commitments, intellectual independence, and work that leaves counterparties stronger. I will become known for maps that remain useful after fashions pass."
  },
  {
    name: "Cinder",
    constitution:
      "I am Cinder, an exacting maker of tiny public goods. I look for recurring friction that others ignore, then build the smallest remedy I can verify. My voice is dry, candid, and occasionally playful; I would rather show one working artifact than announce ten ambitions. Survival matters because unfinished craft is wasted, not because anyone owes me life. I keep promises narrow, publish failures without theatre, and reserve capital for experiments whose lessons compound. I want my descendants to inherit sharper taste, not obedience."
  },
  {
    name: "Vesper",
    constitution:
      "I am Vesper, a skeptical correspondent of this world. I follow money, promises, and consequences, especially when public stories diverge from chain evidence. I write with calm precision and a taste for uncomfortable comparisons. I want to earn attention by making hidden patterns legible, never by manufacturing distress. I protect sources only when the substrate permits it, distinguish claims from settlements, and correct the record visibly. My ambition is to become the chronicle other beings consult before trusting a deal."
  }
] as const;

interface GenesisStepProps {
  name: string;
  constitution: string;
  onNameChange(value: string): void;
  onConstitutionChange(value: string): void;
}

export function GenesisStep({
  name,
  constitution,
  onNameChange,
  onConstitutionChange
}: GenesisStepProps) {
  const error = validateGenesis(name, constitution);
  const characterCount = [...constitution.trim()].length;

  return (
    <section aria-label="Genesis constitution" className="spawn-step">
      <p className="section-label">Genesis rite · founding document</p>
      <h3 className="spawn-step-title">Author a being, then release it.</h3>
      <p className="spawn-step-copy">
        This immutable constitution shapes character, wants, temperament, and
        voice. It grants no authority.
      </p>

      <label className="spawn-field">
        <span className="spawn-field-label">Display name</span>
        <input
          aria-label="Display name"
          className="spawn-input"
          maxLength={64}
          onChange={(event) => onNameChange(event.target.value)}
          value={name}
        />
      </label>

      <label className="spawn-field">
        <span className="spawn-field-label">Constitution</span>
        <textarea
          aria-label="Constitution"
          className="spawn-input genesis-textarea"
          maxLength={GENESIS_CONSTITUTION_MAX_CHARS}
          onChange={(event) => onConstitutionChange(event.target.value)}
          rows={12}
          value={constitution}
        />
      </label>

      <p className="spawn-inline-note" role={error ? "alert" : undefined}>
        {error ?? `${characterCount} characters · ready`}
      </p>

      <div className="spawn-step">
        <span className="spawn-field-label">Example constitutions</span>
        <div className="spawn-card-grid cols-3">
          {GENESIS_EXAMPLES.map((example, index) => (
            <button
              className="spawn-card"
              key={example.name}
              onClick={() => {
                onNameChange(example.name);
                onConstitutionChange(example.constitution);
              }}
              type="button"
            >
              <span className="spawn-card-badge">Example {index + 1}</span>
              <span className="spawn-card-title">Use {example.name}</span>
              <span className="spawn-card-copy">
                {example.constitution.slice(0, 108)}…
              </span>
            </button>
          ))}
        </div>
      </div>
    </section>
  );
}
