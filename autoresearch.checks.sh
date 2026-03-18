#!/bin/bash
set -euo pipefail
cargo test assemble_system_prompt --lib -q
cargo check -q
