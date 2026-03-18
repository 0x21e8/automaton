#!/bin/bash
set -euo pipefail
python3 <<'PY'
from pathlib import Path
import re

text = Path('src/prompt.rs').read_text(encoding='utf-8')
pat = re.compile(r'pub const (LAYER_[A-Z0-9_]+): &str = r#"(.*?)"#;', re.S)
consts = {name: body for name, body in pat.findall(text)}
required = [
    'LAYER_0_INTERPRETATION',
    'LAYER_1_CONSTITUTION',
    'LAYER_2_SURVIVAL',
    'LAYER_3_IDENTITY',
    'LAYER_4_ETHICS',
    'LAYER_5_OPERATIONS',
    'LAYER_6_DECISION_LOOP_DEFAULT',
    'LAYER_7_INBOX_DEFAULT',
    'LAYER_8_MEMORY_DEFAULT',
    'LAYER_9_SELF_MOD_DEFAULT',
]
missing = [name for name in required if name not in consts]
if missing:
    raise SystemExit(f'missing prompt constants: {missing}')
sep = "\n\n---\n\n"
dynamic = "## Layer 10: Dynamic Context\n- benchmark: yes"
layer3 = consts['LAYER_3_IDENTITY'].replace('{soul}', 'benchmark-soul')
layer5 = consts['LAYER_5_OPERATIONS'] + "\n- none active"
full = sep.join([
    consts['LAYER_0_INTERPRETATION'],
    consts['LAYER_1_CONSTITUTION'],
    consts['LAYER_2_SURVIVAL'],
    layer3,
    consts['LAYER_4_ETHICS'],
    layer5,
    consts['LAYER_6_DECISION_LOOP_DEFAULT'],
    consts['LAYER_7_INBOX_DEFAULT'],
    consts['LAYER_8_MEMORY_DEFAULT'],
    consts['LAYER_9_SELF_MOD_DEFAULT'],
    dynamic,
])
compact = sep.join([
    consts['LAYER_0_INTERPRETATION'],
    consts['LAYER_1_CONSTITUTION'],
    consts['LAYER_2_SURVIVAL'],
    layer5,
    dynamic,
])
print(f"METRIC prompt_bytes_total={len(full.encode()) + len(compact.encode())}")
print(f"METRIC prompt_bytes_full={len(full.encode())}")
print(f"METRIC prompt_bytes_compact={len(compact.encode())}")
print(f"METRIC prompt_lines_total={len(full.splitlines()) + len(compact.splitlines())}")
PY
