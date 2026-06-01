#!/bin/bash
set -e

echo "📦 Running NPM packaging audit..."

# Navigate to nanos-sdk directory
cd "$(dirname "$0")"

# Remove any old tgz files
rm -f nanos-sdk-*.tgz

# Run npm pack
echo "📦 Packing nanos-sdk..."
PACK_FILE=$(npm pack 2>&1 | tail -n 1)

echo "📦 Package generated: $PACK_FILE"

# Check if pack exists
if [ ! -f "$PACK_FILE" ]; then
    echo "❌ Error: Package file '$PACK_FILE' was not generated."
    exit 1
fi

# List files inside tarball
echo "📦 Auditing contents of $PACK_FILE:"
CONTENTS=$(tar -tf "$PACK_FILE")
echo "$CONTENTS"

# Check if required files are present
REQUIRED_FILES=("package/package.json" "package/index.js" "package/bin/nanos-compile.js" "package/README.md")
for file in "${REQUIRED_FILES[@]}"; do
    if ! echo "$CONTENTS" | grep -q "$file"; then
        echo "❌ Error: Required file '$file' is missing in the package!"
        exit 1
    fi
done

echo "✅ Integrity check passed: all required files are present in the package tarball."

# Clean up
rm -f "$PACK_FILE"
echo "📦 Cleaned up tarball artifact."
