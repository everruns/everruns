#!/usr/bin/env bash
# Build a labeled email corpus from the SpamAssassin public corpus.
#
# The messages are not vendored: copyright for their text stays with the
# original senders, so this script downloads them into a gitignored cache and
# writes the sampled JSONL beside it. Sampling is deterministic — same counts
# in, same messages out — so a rerun reproduces a published run.
set -euo pipefail

data_dir="$(cd "$(dirname "$0")" && pwd)"
cache_dir="$data_dir/cache"
out="${OUT:-$data_dir/corpus.jsonl}"
base_url="https://spamassassin.apache.org/old/publiccorpus"

# 50 spam, 50 legitimate. The legitimate half is deliberately two thirds
# `hard_ham` — newsletters and receipts that read spammish — because a corpus
# where every legitimate mail is a plain-text mailing list post cannot show a
# routing threshold doing any work.
spam_count="${SPAM_COUNT:-50}"
hard_ham_count="${HARD_HAM_COUNT:-33}"
easy_ham_count="${EASY_HAM_COUNT:-17}"

mkdir -p "$cache_dir"
for archive in 20030228_spam 20030228_hard_ham 20030228_easy_ham; do
  if [[ ! -d "$cache_dir/${archive#*_}" ]]; then
    echo "fetching $archive.tar.bz2"
    curl -fsS -o "$cache_dir/$archive.tar.bz2" "$base_url/$archive.tar.bz2"
    tar xjf "$cache_dir/$archive.tar.bz2" -C "$cache_dir"
  fi
done

SPAM_DIR="$cache_dir/spam" HARD_HAM_DIR="$cache_dir/hard_ham" EASY_HAM_DIR="$cache_dir/easy_ham" \
SPAM_COUNT="$spam_count" HARD_HAM_COUNT="$hard_ham_count" EASY_HAM_COUNT="$easy_ham_count" \
OUT="$out" python3 "$data_dir/build_corpus.py"

echo "wrote $out ($(wc -l < "$out") messages)"
