#!/usr/bin/env bash
# Build the feasibility study PDF with XeLaTeX + bibtex via latexmk.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
latexmk -xelatex -interaction=nonstopmode -halt-on-error main.tex
echo "Output: $(pwd)/main.pdf"
