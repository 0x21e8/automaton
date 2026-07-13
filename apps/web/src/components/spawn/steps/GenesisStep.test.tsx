import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { GenesisStep, GENESIS_EXAMPLES } from "./GenesisStep";

describe("GenesisStep", () => {
  it("is the authoring surface and offers specific example constitutions", () => {
    const markup = renderToStaticMarkup(
      <GenesisStep
        constitution=""
        name=""
        onConstitutionChange={vi.fn()}
        onNameChange={vi.fn()}
      />
    );
    expect(markup).toContain("Author a being, then release it.");
    expect(markup).toContain("Name must be 1–64 characters.");
    expect(markup).toContain("Use Meridian");
    expect(markup).toContain("Example 1");
    expect(GENESIS_EXAMPLES).toHaveLength(3);
    expect([...GENESIS_EXAMPLES[0].constitution].length).toBeGreaterThanOrEqual(400);
  });
});
