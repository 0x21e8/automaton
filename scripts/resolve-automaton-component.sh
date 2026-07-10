#!/bin/sh

set -eu

ROOT_DIR=${AUTOMATON_LAUNCHPAD_ROOT:-$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)}

if [ -n "${AUTOMATON_COMPONENT_ROOT:-}" ]; then
  component_root=$AUTOMATON_COMPONENT_ROOT
elif [ -n "${IC_AUTOMATON_REPO:-}" ]; then
  echo "warning: IC_AUTOMATON_REPO is deprecated; use AUTOMATON_COMPONENT_ROOT" >&2
  component_root=$IC_AUTOMATON_REPO
else
  component_root=$ROOT_DIR/components/ic-automaton
fi

case "$component_root" in
  /*) ;;
  *) component_root="$ROOT_DIR/$component_root" ;;
esac

component_root=$(CDPATH= cd -- "$component_root" 2>/dev/null && pwd) || {
  echo "automaton component root does not exist: $component_root" >&2
  exit 1
}

printf '%s\n' "$component_root"
