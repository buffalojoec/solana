#!/usr/bin/env bash

if ! command -v cargo &> /dev/null ; then
  >&2 echo "Failed to find cargo. Mac readlink doesn't support -f. Consider switching
  to gnu readlink with 'brew install coreutils' and then symlink greadlink as
  /usr/local/bin/readlink."
  exit 1
fi

set -e

shifted_args=()
while [[ -n $1 ]]; do
  if [[ $1 = -- ]]; then
    escape_marker=found
    shift
    break
  elif [[ $1 = "--ignore-exit-code" ]]; then
    ignore=1
    shift
  else
    shifted_args+=("$1")
    shift
  fi
done

# When "--" appear at the first and shifted_args is empty, consume it here
# to unambiguously pass and use any other "--" for cargo
if [[ -n $escape_marker && ${#shifted_args[@]} -gt 0 ]]; then
  files="${shifted_args[*]}"
  for file in $files; do
    if [[ $file = "${file%Cargo.lock}" ]]; then
      echo "$0: unrecognizable as Cargo.lock path (prepend \"--\"?): $file" >&2
      exit 1
    fi
  done
  shifted_args=()
else
  files="$(git ls-files :**Cargo.lock)"
fi

for lock_file in $files; do
  if [[ $lock_file = *ci/xtask/tests/dummy-workspace* ]]; then
    continue
  fi

  if [[ -n $CI ]]; then
    echo "--- [$lock_file]: cargo " "${shifted_args[@]}" "$@"
  fi

  # When running `cargo fmt --all`, replace --all with explicit workspace
  # packages to avoid formatting local path dependencies (e.g. ../sbpf).
  local_args=("$@")
  is_fmt=false
  for arg in "${shifted_args[@]}" "${local_args[@]}"; do
    if [[ $arg = "fmt" ]]; then
      is_fmt=true
      break
    fi
  done
  if $is_fmt; then
    new_args=()
    for arg in "${local_args[@]}"; do
      if [[ $arg = "--all" ]]; then
        # Replace --all with per-workspace-member -p flags
        lock_dir="$(dirname "$lock_file")"
        while IFS= read -r pkg; do
          new_args+=("-p" "$pkg")
        done < <(cargo metadata --no-deps --format-version 1 --manifest-path "$lock_dir/Cargo.toml" 2>/dev/null | python3 -c "import sys,json; [print(p['name']) for p in json.load(sys.stdin)['packages']]")
      else
        new_args+=("$arg")
      fi
    done
    local_args=("${new_args[@]}")
  fi
  if (set -x && cd "$(dirname "$lock_file")" && cargo "${shifted_args[@]}" "${local_args[@]}"); then
    # noop
    true
  else
    failed_exit_code=$?
    if [[ -n $ignore ]]; then
      echo "$0: WARN: ignoring last cargo command failed exit code as requested:" $failed_exit_code
      true
    else
      exit $failed_exit_code
    fi
  fi
done
