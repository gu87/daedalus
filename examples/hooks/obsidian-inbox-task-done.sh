#!/bin/sh
# Obsidian inbox hook — append run summaries to an Obsidian inbox file.
#
# Set OBSIDIAN_DAEDALUS_INBOX to the target Markdown path, for example:
#   export OBSIDIAN_DAEDALUS_INBOX="$HOME/个人知识库/0-inbox/daedalus-runs.md"
#
# The daemon sets these environment variables:
#   DAEDALUS_RUN_ID  DAEDALUS_TASK_ID  DAEDALUS_RUN_DIR
#   DAEDALUS_TRANSCRIPT_PATH  DAEDALUS_SUMMARY_PATH  DAEDALUS_STATUS

set -eu

INBOX="${OBSIDIAN_DAEDALUS_INBOX:-}"

if [ -z "$INBOX" ]; then
  echo "obsidian-inbox: OBSIDIAN_DAEDALUS_INBOX is not set — skipping" >&2
  exit 0
fi

if [ ! -f "$DAEDALUS_SUMMARY_PATH" ]; then
  echo "obsidian-inbox: summary not found at $DAEDALUS_SUMMARY_PATH" >&2
  exit 1
fi

mkdir -p "$(dirname "$INBOX")"

DATE="$(date '+%Y-%m-%d %H:%M')"

{
  echo ""
  echo "## $DATE $DAEDALUS_RUN_ID"
  echo ""
  echo "Source: $DAEDALUS_SUMMARY_PATH"
  echo ""
  cat "$DAEDALUS_SUMMARY_PATH"
} >> "$INBOX"

echo "obsidian-inbox: appended $DAEDALUS_RUN_ID to $INBOX"
