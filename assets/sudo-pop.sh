# Installed by `sudo-pop --init`. Inherited by agents and scripts launched here.
# Remove only our old alias; leave other aliases and shell functions alone.
case "$(alias sudo 2>/dev/null)" in
  "alias sudo='sudo-pop'"|"sudo=sudo-pop"|"sudo='sudo-pop'") unalias sudo ;;
esac

# Put the wrapper first, removing old occurrences when the rc is sourced again.
# Preserve other entries (including empty entries) and their order.
__sudo_pop_bin="$HOME/.local/lib/sudo-pop/bin"
__sudo_pop_path="$__sudo_pop_bin"
__sudo_pop_rest="${PATH-}"
while :; do
  __sudo_pop_part=${__sudo_pop_rest%%:*}
  if [ "$__sudo_pop_part" != "$__sudo_pop_bin" ]; then
    __sudo_pop_path="$__sudo_pop_path:$__sudo_pop_part"
  fi
  case "$__sudo_pop_rest" in
    *:*) __sudo_pop_rest=${__sudo_pop_rest#*:} ;;
    *) break ;;
  esac
done
export PATH="$__sudo_pop_path"
unset __sudo_pop_bin __sudo_pop_path __sudo_pop_rest __sudo_pop_part
hash -r 2>/dev/null || true
