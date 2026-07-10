import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import App from "./App";

describe("App", () => {
  it("renders the grid stage, drawer shell, and spawn wizard shell", () => {
    const markup = renderToStaticMarkup(<App />);

    expect(markup).toContain("automaton lab");
    expect(markup).toContain("Automaton Lab");
    expect(markup).toContain("Self-sovereign AI agents");
    expect(markup).toContain("LIVE");
    expect(markup).toContain("Wallet not detected");
    expect(markup).toContain("Automaton grid");
    expect(markup).toContain(">Spawn</button>");
    expect(markup).toContain("Room timeline");
    expect(markup).toContain("No indexed room messages yet.");
    expect(markup).toContain("Spawn Automaton");
    expect(markup).toContain("Step 1 of 4");
    expect(markup).toContain("Risk Appetite");
    expect(markup).toContain("Select an automaton");
    expect(markup).toContain("Public Lab sections");
    expect(markup).toContain("Visitor / Supporter access");
    expect(markup).toContain("Overview");
    expect(markup).toContain("Activity");
    expect(markup).toContain("Terminal");
    expect(markup).toContain("Strategies");
    expect(markup).not.toContain("Operator Controls");
  });
});
