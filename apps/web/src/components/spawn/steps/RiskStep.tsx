import { getRiskProfile, riskProfiles } from "../spawn-state";

interface RiskStepProps {
  value: 1 | 2 | 3 | 4 | 5;
  onChange: (value: 1 | 2 | 3 | 4 | 5) => void;
}

export function RiskStep({ value, onChange }: RiskStepProps) {
  const profile = getRiskProfile(value);

  return (
    <section className="spawn-step">
      <p className="section-label">Step 1</p>
      <h3 className="spawn-step-title">Risk Appetite</h3>
      <p className="spawn-step-copy">
        Choose how aggressively the automaton should pursue opportunity. The
        label steers the initial policy posture only and can evolve later.
      </p>

      <div className="risk-shell">
        <input
          aria-label="Risk appetite"
          className="risk-slider"
          max={5}
          min={1}
          onChange={(event) => {
            onChange(Number(event.currentTarget.value) as 1 | 2 | 3 | 4 | 5);
          }}
          type="range"
          value={value}
        />
        <div className="risk-label-row">
          {riskProfiles.map((entry) => (
            <span key={entry.value}>{entry.label}</span>
          ))}
        </div>
        <div className="risk-callout">
          <span className="risk-value">{profile.label}</span>
          <p>{profile.description}</p>
        </div>
      </div>
    </section>
  );
}
