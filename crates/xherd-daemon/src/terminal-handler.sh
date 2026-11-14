#!/bin/sh
# xherd-open-terminal: opens your own terminal app in an xherd agent's
# worktree, from an `xherd-term://open?dir=<dir>&terminal=<id>` link (R-707).
#
#   xherd-open-terminal open <url>    open the terminal (also: <url> alone)
#   xherd-open-terminal terminals     the terminals found here, default first
#
# Any web page can fire such a link, so the URL is data: `dir` is accepted
# only when it is an agent worktree itself (`<root>/<repo>/<agent>`, with a
# `.git` file and a `.xherd/` folder, no symlink leading elsewhere), and is
# only ever used as a working directory or one quoted argument. Nothing from
# the URL is run, evaluated or put in a command line.
#
# XHERD_TERM_DRY_RUN=1 prints the command, one word per line, instead of
# running it; XHERD_TERM_OS=mac|linux forces the OS. Both are for tests.
#
# Installed by xherd (Settings -> Open in); edits are lost on the next install.

set -u
unset CDPATH

dry=${XHERD_TERM_DRY_RUN:-}
os=${XHERD_TERM_OS:-}
if [ -z "$os" ]; then
	case $(uname -s 2>/dev/null) in
	Darwin) os=mac ;;
	*) os=linux ;;
	esac
fi
case $os in mac | linux) ;; *) os=linux ;; esac

MAC_TERMINALS='terminal iterm ghostty'
LINUX_TERMINALS='ghostty konsole gnome-terminal kitty alacritty wezterm xfce4-terminal foot xterm xdg-terminal-exec x-terminal-emulator'

# Says why a request is refused (stderr, and a desktop notification when one
# can be shown) and stops. The message is always one of this file's own
# texts, never anything from the URL.
refuse() {
	printf 'xherd-open-terminal: %s\n' "$1" >&2
	if [ -z "$dry" ]; then
		if [ "$os" = mac ]; then
			if command -v osascript >/dev/null 2>&1; then
				osascript -e "display notification \"$1\" with title \"xherd\"" >/dev/null 2>&1
			fi
		elif command -v notify-send >/dev/null 2>&1; then
			notify-send "xherd" "$1" >/dev/null 2>&1
		fi
	fi
	exit 1
}

known() { # <id>: in this OS's list
	if [ "$os" = mac ]; then list=$MAC_TERMINALS; else list=$LINUX_TERMINALS; fi
	for k in $list; do
		[ "$k" = "$1" ] && return 0
	done
	return 1
}

mac_app() { # <id>: the app bundle that shows it is installed
	case $1 in
	iterm) echo iTerm.app ;;
	ghostty) echo Ghostty.app ;;
	*) echo Terminal.app ;;
	esac
}

installed() { # <id>: known for this OS and present
	known "$1" || return 1
	if [ "$os" = mac ]; then
		[ "$1" = terminal ] && return 0
		app=$(mac_app "$1")
		[ -d "/Applications/$app" ] || [ -d "$HOME/Applications/$app" ]
	else
		command -v "$1" >/dev/null 2>&1
	fi
}

# A terminal named by a command line (`/usr/bin/kitty -1`, `'konsole'`) when
# it is a known one that is installed.
named() {
	first=$(printf '%s\n' "$1" | sed -n "1{s/^[[:space:]']*//;s/[[:space:]'].*//;s#.*/##;p;}")
	[ -n "$first" ] && installed "$first" && echo "$first"
}

# KDE Plasma: `[General]` `TerminalApplication=kitty` (wins) or
# `TerminalService=org.kde.konsole.desktop`.
kde_terminal() {
	file=${XDG_CONFIG_HOME:-$HOME/.config}/kdeglobals
	[ -f "$file" ] || return 1
	app=$(LC_ALL=C awk -F= '
		/^\[/ { general = ($0 == "[General]"); next }
		general && $1 == "TerminalApplication" && app == "" { app = $2 }
		END { print app }' "$file")
	if [ -n "$app" ] && named "$app"; then return 0; fi
	service=$(LC_ALL=C awk -F= '
		/^\[/ { general = ($0 == "[General]"); next }
		general && $1 == "TerminalService" && svc == "" { svc = tolower($2) }
		END { print svc }' "$file")
	[ -n "$service" ] || return 1
	for k in $LINUX_TERMINALS; do
		case $service in
		*"$k"*) installed "$k" && echo "$k" && return 0 ;;
		esac
	done
	return 1
}

default_terminal() {
	if [ "$os" = mac ]; then
		echo terminal
		return 0
	fi
	if [ -n "${TERMINAL:-}" ] && named "$TERMINAL"; then return 0; fi
	if kde_terminal; then return 0; fi
	if command -v gsettings >/dev/null 2>&1; then
		g=$(gsettings get org.gnome.desktop.default-applications.terminal exec 2>/dev/null)
		if [ -n "$g" ] && named "$g"; then return 0; fi
	fi
	for k in $LINUX_TERMINALS; do
		installed "$k" && echo "$k" && return 0
	done
	return 1
}

list_terminals() {
	d=$(default_terminal) || d=
	[ -n "$d" ] && echo "$d"
	if [ "$os" = mac ]; then list=$MAC_TERMINALS; else list=$LINUX_TERMINALS; fi
	for k in $list; do
		[ "$k" != "$d" ] && installed "$k" && echo "$k"
	done
	return 0
}

# Percent-decodes one raw query value. Prints it as a printf format made only
# of octal escapes (`\101…`), or fails on a character outside the URL charset,
# a bad escape or a decoded control character, so a newline or NUL never
# reaches a variable. Byte by byte under LC_ALL=C, so UTF-8 passes through.
decode() {
	printf '%s\n' "$1" | LC_ALL=C awk '
		BEGIN {
			ok = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789%._~/!$\047()*+,;=:@-"
			for (i = 0; i < 256; i++) hex[sprintf("%02X", i)] = i
			for (i = 32; i < 127; i++) ord[sprintf("%c", i)] = i
		}
		NR > 1 { exit 2 }
		{
			s = $0; n = length(s); out = ""
			for (i = 1; i <= n; ) {
				c = substr(s, i, 1)
				if (index(ok, c) == 0) exit 2
				if (c == "%") {
					x = toupper(substr(s, i + 1, 2))
					if (length(x) != 2 || !(x in hex)) exit 2
					b = hex[x]; i += 3
				} else {
					b = ord[c]; i++
				}
				if (b < 32 || b == 127) exit 2
				out = out sprintf("\\%03o", b)
			}
			printf "%s", out
		}'
}

# Runs (or, dry, prints) the command, detached from the browser.
launch() {
	if [ -n "$dry" ]; then
		for w in "$@"; do
			printf '%s\n' "$w"
		done
		exit 0
	fi
	if [ "$os" = mac ]; then
		"$@" </dev/null >/dev/null 2>&1 || refuse "the terminal app could not be started."
	elif command -v setsid >/dev/null 2>&1; then
		setsid "$@" </dev/null >/dev/null 2>&1 &
	else
		nohup "$@" </dev/null >/dev/null 2>&1 &
	fi
	exit 0
}

open_url() {
	url=$1
	[ ${#url} -le 8192 ] || refuse "the link is too long."
	case $url in
	xherd-term://open\?*) query=${url#xherd-term://open\?} ;;
	xherd-term://open/\?*) query=${url#xherd-term://open/\?} ;;
	*) refuse "this is not an xherd terminal link." ;;
	esac

	dir_raw=
	have_dir=
	term_raw=
	have_term=
	old_ifs=$IFS
	IFS='&'
	set -f
	# shellcheck disable=SC2086 # split on & on purpose, globbing off
	set -- $query
	set +f
	IFS=$old_ifs
	for pair in "$@"; do
		case $pair in
		dir=*)
			[ -z "$have_dir" ] || refuse "the link names the folder twice."
			have_dir=1
			dir_raw=${pair#dir=}
			;;
		terminal=*)
			[ -z "$have_term" ] || refuse "the link names the terminal twice."
			have_term=1
			term_raw=${pair#terminal=}
			;;
		esac
	done
	[ -n "$have_dir" ] || refuse "the link names no folder."

	fmt=$(decode "$dir_raw") || refuse "the link's folder is not a valid path."
	# shellcheck disable=SC2059 # the format holds octal escapes only
	dir=$(printf "$fmt")
	terminal=
	if [ -n "$have_term" ]; then
		fmt=$(decode "$term_raw") || refuse "the link's terminal is not valid."
		# shellcheck disable=SC2059
		terminal=$(printf "$fmt")
	fi

	conf=$HOME/.config/xherd/terminal-handler.conf
	[ -f "$conf" ] || refuse "the handler is not set up: install it again from Settings -> Open in."
	root=$(sed -n 's/^root=//p' "$conf" | sed -n 1p)
	while [ "${root%/}" != "$root" ]; do root=${root%/}; done
	case $root in
	/?*) ;;
	*) refuse "the handler is not set up: install it again from Settings -> Open in." ;;
	esac
	root_real=$(cd -P -- "$root" 2>/dev/null && pwd -P) ||
		refuse "the worktrees folder is missing: install the handler again from Settings -> Open in."

	case $dir in
	/*) ;;
	*) refuse "the folder is not an absolute path." ;;
	esac
	rel=${dir#"$root"/}
	[ "$rel" != "$dir" ] || refuse "the folder is not an xherd agent worktree."
	case $rel in
	*/*/* | */) refuse "the folder is not an xherd agent worktree." ;;
	?*/?*) ;;
	*) refuse "the folder is not an xherd agent worktree." ;;
	esac
	repo=${rel%%/*}
	agent=${rel#*/}
	for seg in "$repo" "$agent"; do
		case $seg in
		. | .. | -*) refuse "the folder is not an xherd agent worktree." ;;
		esac
	done
	[ -d "$dir" ] && [ ! -L "$dir" ] || refuse "the folder does not exist."
	real=$(cd -P -- "$dir" 2>/dev/null && pwd -P) || refuse "the folder does not exist."
	[ "$real" = "$root_real/$repo/$agent" ] || refuse "the folder is not an xherd agent worktree."
	[ -f "$dir/.git" ] && [ ! -L "$dir/.git" ] || refuse "the folder is not an xherd agent worktree."
	[ -d "$dir/.xherd" ] && [ ! -L "$dir/.xherd" ] || refuse "the folder is not an xherd agent worktree."

	if [ -z "$terminal" ] || ! installed "$terminal"; then
		terminal=$(default_terminal) || refuse "no terminal app was found."
	fi

	if [ "$os" = mac ]; then
		case $terminal in
		iterm) launch open -a iTerm "$dir" ;;
		ghostty) launch open -na Ghostty --args "--working-directory=$dir" ;;
		*) launch open -a Terminal "$dir" ;;
		esac
	fi
	cd -- "$dir" || refuse "the folder does not exist."
	case $terminal in
	ghostty) launch ghostty "--working-directory=$dir" ;;
	konsole) launch konsole --workdir "$dir" ;;
	gnome-terminal) launch gnome-terminal "--working-directory=$dir" ;;
	kitty) launch kitty --directory "$dir" ;;
	alacritty) launch alacritty --working-directory "$dir" ;;
	wezterm) launch wezterm start --cwd "$dir" ;;
	xfce4-terminal) launch xfce4-terminal "--working-directory=$dir" ;;
	foot) launch foot "--working-directory=$dir" ;;
	*) launch "$terminal" ;;
	esac
}

: "${HOME:?}"
case ${1:-} in
terminals) list_terminals ;;
open)
	[ $# -eq 2 ] || refuse "usage: xherd-open-terminal open <url>"
	open_url "$2"
	;;
xherd-term:*)
	[ $# -eq 1 ] || refuse "usage: xherd-open-terminal open <url>"
	open_url "$1"
	;;
*) refuse "usage: xherd-open-terminal open <url> | terminals" ;;
esac
