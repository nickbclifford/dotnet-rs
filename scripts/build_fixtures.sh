#!/usr/bin/env bash

# Mirror CI build-fixtures logic for local use.
# Accepts an optional --output-dir argument.

set -e

# Default output directory as in CI
OUTPUT_DIR="target/debug/dotnet-fixtures"

# Parse arguments
while [[ $# -gt 0 ]]; do
  case $1 in
    --output-dir)
      OUTPUT_DIR="$2"
      shift # past argument
      shift # past value
      ;;
    *)
      echo "Unknown option: $1"
      echo "Usage: $0 [--output-dir <directory>]"
      exit 1
      ;;
  esac
done

# Ensure output directory exists
mkdir -p "$OUTPUT_DIR"

# Get absolute paths
ABS_OUTPUT_DIR=$(readlink -f "$OUTPUT_DIR")
PROJECT_ROOT=$(pwd)

# Define paths relative to the output directory as in CI/build.rs
MSBUILD_OBJ="$ABS_OUTPUT_DIR/msbuild-obj/"
MSBUILD_BIN="$ABS_OUTPUT_DIR/msbuild-bin/"

echo "Building .NET fixtures..."
echo "  Output dir: $ABS_OUTPUT_DIR"

# Mirror CI logic for building fixtures
dotnet restore crates/dotnet-cli/tests/SingleFile.csproj \
    -p:BaseIntermediateOutputPath="$MSBUILD_OBJ" \
    -p:BaseOutputPath="$MSBUILD_BIN" \
    --nologo -v:q

dotnet build crates/dotnet-cli/tests/BatchFixtures.csproj \
    -p:FixtureOutputBase="$ABS_OUTPUT_DIR/" \
    -p:BaseIntermediateOutputPath="$MSBUILD_OBJ" \
    -p:BaseOutputPath="$MSBUILD_BIN" \
    -m -v:q --nologo -clp:ErrorsOnly --no-restore

echo "Fixtures built successfully."
echo "To use these fixtures in tests, run:"
echo "  DOTNET_USE_PREBUILT_FIXTURES=1 cargo test"
