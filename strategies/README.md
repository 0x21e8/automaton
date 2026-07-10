# Canonical strategy assets

Each strategy directory contains the executable `recipe.json` and its
display/provenance `metadata.json`. `manifest.json` is the factory seed
manifest; `npm run verify:strategies` checks that the recipe, metadata, and
factory embedding remain aligned.

The recipes originate from the imported `ic-automaton` component. The root
tree is the canonical workspace path used by the factory and release tooling.
