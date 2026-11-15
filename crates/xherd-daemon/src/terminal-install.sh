#!/bin/sh
# Installs (or, with `uninstall`, removes) xherd's `xherd-term://` link
# handler, which opens your own terminal app in an agent's worktree (R-707):
#
#   curl -fsSL <xherd>/api/v1/open-in/terminal-handler.sh | sh
#   curl -fsSL <xherd>/api/v1/open-in/terminal-handler.sh | sh -s uninstall
#
# Linux: ~/.local/bin/xherd-open-terminal and a desktop entry for the
# scheme. macOS: ~/Library/Application Support/xherd/ holds the handler and
# XherdTerminal.app, a small AppleScript app registered for the scheme.
# Then it tells xherd what it installed, so the Terminal entry shows.
#
# Rendered by xherd for this computer: the worktrees folder and xherd's
# address below are baked in. XHERD_TERM_DRY_RUN=1 writes the files but
# prints the registration commands instead of running them;
# XHERD_TERM_OS=mac|linux forces the OS.

set -u
unset CDPATH

root='@ROOT@'
report_url='@REPORT_URL@'
handler_version='@VERSION@'
xherd_version='@XHERD@'

PLISTBUDDY=/usr/libexec/PlistBuddy
LSREGISTER=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
BUNDLE_ID=io.github.xionjames.xherd.terminal

say() { printf 'xherd: %s\n' "$1" >&2; }
die() {
	say "$1"
	exit 1
}

# Runs a command, or prints it on one line in a dry run.
run() {
	if [ -n "$dry" ]; then
		printf '%s\n' "$*"
		return 0
	fi
	"$@" </dev/null
}

# Writes stdin to <file> with <mode>, through a temporary file and `mv`.
put() {
	tmp="$1.tmp.$$"
	if cat >"$tmp" && chmod "$2" "$tmp" && mv -f "$tmp" "$1"; then
		return 0
	fi
	rm -f "$tmp"
	die "could not write $1"
}

write_handler() {
	cat <<'XHERD_HANDLER_EOF'
@HANDLER@
XHERD_HANDLER_EOF
}

write_conf() {
	printf 'root=%s\nversion=%s\n' "$root" "$handler_version" | put "$conf" 600
}

# Tells xherd what is installed: the JSON is built only from this file's own
# values and the handler's fixed terminal ids. A failure is only a warning.
report() {
	body=$1
	url="$report_url/api/v1/open-in/terminal-handler"
	if command -v curl >/dev/null 2>&1 || [ -n "$dry" ]; then
		run curl -fsS -m 10 -o /dev/null -H 'Content-Type: application/json' -d "$body" "$url" && return 0
	elif command -v wget >/dev/null 2>&1; then
		run wget -q -T 10 -O /dev/null --header='Content-Type: application/json' --post-data="$body" "$url" && return 0
	fi
	say "done, but xherd was not told: run this again once the daemon is up."
	return 0
}

report_installed() {
	terms=
	for t in $("$handler" terminals </dev/null 2>/dev/null); do
		case $t in
		*[!a-z0-9-]* | '') continue ;;
		esac
		terms="$terms${terms:+,}\"$t\""
	done
	report "{\"version\":$handler_version,\"xherd\":\"$xherd_version\",\"os\":\"$os\",\"terminals\":[$terms],\"root\":\"$root\"}"
}

install_linux() {
	mkdir -p "$HOME/.local/bin" "$apps" || die "could not create the folders"
	write_handler | put "$handler" 755
	write_conf
	{
		printf '[Desktop Entry]\n'
		printf 'Type=Application\n'
		printf 'Name=xherd terminal link\n'
		printf 'Comment=Opens a terminal in an xherd agent worktree\n'
		printf 'Exec="%s" open %%u\n' "$handler"
		printf 'Terminal=false\n'
		printf 'NoDisplay=true\n'
		printf 'MimeType=x-scheme-handler/xherd-term;\n'
	} | put "$desktop" 644
	if [ -n "$dry" ] || command -v xdg-mime >/dev/null 2>&1; then
		run xdg-mime default xherd-term.desktop x-scheme-handler/xherd-term ||
			say "xdg-mime failed: your desktop may still find the entry by itself."
	else
		say "xdg-mime was not found: your desktop may still find the entry by itself."
	fi
	if [ -n "$dry" ] || command -v update-desktop-database >/dev/null 2>&1; then
		run update-desktop-database "$apps" || true
	fi
}

uninstall_linux() {
	rm -f "$handler" "$conf" "$desktop"
	mimeapps=${XDG_CONFIG_HOME:-$HOME/.config}/mimeapps.list
	if [ -f "$mimeapps" ] && [ -n "$(sed -n '/^x-scheme-handler\/xherd-term=/p' "$mimeapps")" ]; then
		sed '/^x-scheme-handler\/xherd-term=/d' "$mimeapps" | put "$mimeapps" 644
	fi
	if [ -n "$dry" ] || command -v update-desktop-database >/dev/null 2>&1; then
		run update-desktop-database "$apps" || true
	fi
}

install_mac() {
	mkdir -p "$support" || die "could not create $support"
	write_handler | put "$handler" 755
	write_conf
	rm -rf "$app"
	run osacompile -o "$app" \
		-e 'on open location u' \
		-e "do shell script quoted form of \"$handler\" & \" open \" & quoted form of u & \" >/dev/null 2>&1 &\"" \
		-e 'end open location' || die "osacompile failed"
	plist="$app/Contents/Info.plist"
	run "$PLISTBUDDY" -c "Set :CFBundleIdentifier $BUNDLE_ID" "$plist" ||
		run "$PLISTBUDDY" -c "Add :CFBundleIdentifier string $BUNDLE_ID" "$plist" ||
		die "could not set the app's identifier"
	run "$PLISTBUDDY" -c "Add :LSUIElement bool true" "$plist" || die "could not hide the app"
	run "$PLISTBUDDY" -c "Add :CFBundleURLTypes array" \
		-c "Add :CFBundleURLTypes:0 dict" \
		-c "Add :CFBundleURLTypes:0:CFBundleURLName string $BUNDLE_ID" \
		-c "Add :CFBundleURLTypes:0:CFBundleURLSchemes array" \
		-c "Add :CFBundleURLTypes:0:CFBundleURLSchemes:0 string xherd-term" \
		"$plist" || die "could not register the scheme in the app"
	run "$LSREGISTER" -f "$app" || die "lsregister failed"
}

uninstall_mac() {
	if [ -d "$app" ] || [ -n "$dry" ]; then
		run "$LSREGISTER" -u "$app" || true
	fi
	rm -rf "$app"
	rm -f "$handler" "$conf"
	rmdir "$support" 2>/dev/null || true
}

main() {
	: "${HOME:?HOME is not set}"
	dry=${XHERD_TERM_DRY_RUN:-}
	os=${XHERD_TERM_OS:-}
	if [ -z "$os" ]; then
		case $(uname -s 2>/dev/null) in
		Darwin) os=mac ;;
		*) os=linux ;;
		esac
	fi
	case $os in mac | linux) ;; *) os=linux ;; esac

	conf=$HOME/.config/xherd/terminal-handler.conf
	if [ "$os" = mac ]; then
		support="$HOME/Library/Application Support/xherd"
		handler=$support/xherd-open-terminal
		app=$support/XherdTerminal.app
	else
		apps=${XDG_DATA_HOME:-$HOME/.local/share}/applications
		handler=$HOME/.local/bin/xherd-open-terminal
		desktop=$apps/xherd-term.desktop
	fi
	# The handler's path goes into a desktop entry or an AppleScript string.
	case $handler in
	*[\"\`\$\\%]*) die "your home folder's path has a character the link handler cannot be registered with" ;;
	esac

	case ${1:-install} in
	install)
		mkdir -p "$HOME/.config/xherd" || die "could not create $HOME/.config/xherd"
		"install_$os"
		say "installed the xherd-term:// link handler ($handler)."
		report_installed
		;;
	uninstall)
		"uninstall_$os"
		say "removed the xherd-term:// link handler."
		report '{"installed":false}'
		;;
	*) die "usage: sh terminal-handler.sh [install | uninstall]" ;;
	esac
}

main "$@"
