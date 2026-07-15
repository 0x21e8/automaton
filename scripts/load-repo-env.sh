#!/bin/sh

ENV_FILE=${REPO_ENV_FILE:-"$ROOT_DIR/.env"}

trim_whitespace() {
  printf '%s' "$1" | sed 's/^[[:space:]]*//; s/[[:space:]]*$//'
}

if [ ! -f "$ENV_FILE" ]; then
  return 0 2>/dev/null || exit 0
fi

while IFS= read -r raw_line || [ -n "$raw_line" ]; do
  line=$(trim_whitespace "$raw_line")

  case "$line" in
    ""|\#*)
      continue
      ;;
    export\ *)
      line=${line#export }
      ;;
  esac

  case "$line" in
    *=*)
      ;;
    *)
      continue
      ;;
  esac

  key=$(trim_whitespace "${line%%=*}")
  value=$(trim_whitespace "${line#*=}")

  case "$key" in
    ""|*[!A-Za-z0-9_]*)
      continue
      ;;
  esac

  eval "already_set=\${$key+x}"
  if [ "$already_set" = "x" ]; then
    continue
  fi

  case "$value" in
    \"*\")
      value=${value#\"}
      value=${value%\"}
      ;;
    \'*\')
      value=${value#\'}
      value=${value%\'}
      ;;
  esac

  export "$key=$value"
done < "$ENV_FILE"
