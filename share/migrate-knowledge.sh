#!/bin/bash
# RFC-003 Knowledge Migration Script
#
# Migrates Space-scoped knowledge directories to the global
# <oxios home>/knowledge/ directory.
#
# Before RFC-003: each Space had its own knowledge/ directory
#   <oxios home>/workspace/sessions/<space-id>/knowledge/
#
# After RFC-003: single global knowledge base
#   <oxios home>/knowledge/
#
# Usage:
#   bash share/migrate-knowledge.sh          # dry run (shows what would be moved)
#   bash share/migrate-knowledge.sh --run    # actually move files

set -euo pipefail

# Resolve the oxios home — mirrors oxios_kernel::oxi_home:
#   $OXIOS_HOME > $OXI_HOME/oxios > $HOME/.oxi/oxios
if [ -n "${OXIOS_HOME:-}" ]; then
    OXIOS_ROOT="$OXIOS_HOME"
elif [ -n "${OXI_HOME:-}" ]; then
    OXIOS_ROOT="${OXI_HOME%/}/oxios"
else
    OXIOS_ROOT="$HOME/.oxi/oxios"
fi
GLOBAL_KB="${OXIOS_ROOT}/knowledge"
WORKSPACE="${OXIOS_ROOT}/workspace"
DRY_RUN=true

if [[ "${1:-}" == "--run" ]]; then
    DRY_RUN=false
fi

echo "=== RFC-003 Knowledge Migration ==="
echo "Global knowledge base: ${GLOBAL_KB}"
echo ""

# Create global directory
if [[ ! -d "${GLOBAL_KB}" ]]; then
    if $DRY_RUN; then
        echo "[DRY RUN] Would create: ${GLOBAL_KB}"
    else
        mkdir -p "${GLOBAL_KB}"
        echo "Created: ${GLOBAL_KB}"
    fi
fi

# Find Space-scoped knowledge directories
FOUND=0
MOVED=0

if [[ -d "${WORKSPACE}" ]]; then
    while IFS= read -r -d '' space_dir; do
        space_knowledge="${space_dir}/knowledge"
        if [[ -d "${space_knowledge}" ]]; then
            space_name=$(basename "$(dirname "${space_dir}")")
            FOUND=$((FOUND + 1))
            echo "Found Space knowledge: ${space_knowledge}"

            # Copy files (not move, to preserve original as backup)
            if $DRY_RUN; then
                echo "[DRY RUN] Would rsync: ${space_knowledge}/ → ${GLOBAL_KB}/"
            else
                rsync -av --ignore-existing "${space_knowledge}/" "${GLOBAL_KB}/"
                MOVED=$((MOVED + 1))
                echo "  → Merged into ${GLOBAL_KB}"
            fi
        fi
    done < <(find "${WORKSPACE}" -type d -name "knowledge" -print0 2>/dev/null)
fi

echo ""
echo "=== Summary ==="
echo "Space knowledge dirs found: ${FOUND}"
if $DRY_RUN; then
    echo "Mode: DRY RUN (no changes made)"
    echo "Run with --run to apply"
else
    echo "Directories merged: ${MOVED}"
    echo ""
    echo "Original Space directories preserved as backup."
    echo "Remove manually after verifying: find ${WORKSPACE} -type d -name 'knowledge' -exec rm -rf {} +"
fi
