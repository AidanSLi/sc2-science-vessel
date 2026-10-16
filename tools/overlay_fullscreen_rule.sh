#!/usr/bin/env bash
# Puts the app's window above fullscreen windows on KDE Plasma Wayland, using a KWin window
# rule.  The overlay otherwise cannot cover a fullscreen game: KWin's Window::belongsToLayer()
# tests isActiveFullScreen() before keepAbove(), and stacking order sorts purely by layer index,
# so nothing a keep-above toplevel can do reaches above the ActiveLayer.  OverlayLayer can, and
# a window rule is the only way to request it without a compositor patch.
#
# What this costs: OverlayLayer is above *every* fullscreen window, including other
# applications'.  That is a desktop-wide change, so it is opt-in and never applied by the app.
#
# Usage:
#   tools/overlay_fullscreen_rule.sh install   # add the rule and reload KWin
#   tools/overlay_fullscreen_rule.sh remove    # delete it and reload KWin
#   tools/overlay_fullscreen_rule.sh status    # show whether it is present
#
# The class string is the part that is easy to get wrong: KWin matches WM_CLASS
# case-sensitively and the app reports "Sc2-science-vessel" (capital S) as its resource class.
# A lowercase rule matches the resource *name* instead and silently never fires.  See
# docs/adr/0011-yield-the-overlay-to-a-fullscreen-client.md.

set -euo pipefail

CLASS="Sc2-science-vessel"
GROUP="sc2-science-vessel-overlay"
DESCRIPTION="sc2-science-vessel overlay above fullscreen"

# KWin groups rules by number in kwinrulesrc, with [General] listing them.  A named helper
# script cannot know which numbers are free without reading the file, so the rule is written
# into a group named after this tool and the count is recomputed from the existing groups.
config_file() {
	# kwriteconfig6 writes to the session's config dir, honouring XDG_CONFIG_HOME.
	printf '%s\n' "${XDG_CONFIG_HOME:-$HOME/.config}/kwinrulesrc"
}

read_rule_numbers() {
	# Every `[N]` section except [General] is a rule.
	sed -n 's/^\[\([0-9]\+\)\]$/\1/p' "$(config_file)" 2>/dev/null | sort -n
}

install_rule() {
	local file next
	file="$(config_file)"
	next="$(read_rule_numbers | tail -1)"
	next=$(( ${next:-0} + 1 ))

	kwriteconfig6 --file kwinrulesrc --group "$next" --key Description "$DESCRIPTION"
	kwriteconfig6 --file kwinrulesrc --group "$next" --key layer overlay
	kwriteconfig6 --file kwinrulesrc --group "$next" --key layerrule 2
	kwriteconfig6 --file kwinrulesrc --group "$next" --key wmclass "$CLASS"
	kwriteconfig6 --file kwinrulesrc --group "$next" --key wmclassmatch 1
	kwriteconfig6 --file kwinrulesrc --group "$next" --key wmclasscomplete 0

	local all
	all="$(read_rule_numbers | sort -n | paste -sd, -)"
	kwriteconfig6 --file kwinrulesrc --group General --key rules "$all"
	kwriteconfig6 --file kwinrulesrc --group General --key count "$(read_rule_numbers | wc -l)"

	qdbus6 org.kde.KWin /KWin org.kde.KWin.reconfigure >/dev/null 2>&1 || true
	echo "Installed rule $next: $DESCRIPTION (layer=overlay, class=$CLASS)"
	echo "Restart the app, or retoggle the overlay, for it to take effect."
}

remove_rule() {
	local file numbers
	file="$(config_file)"
	numbers="$(read_rule_numbers)"
	if [ -z "$numbers" ]; then
		echo "No rules to remove."
		return
	fi
	for n in $numbers; do
		if grep -q "^Description=$DESCRIPTION$" <(sed -n "/^\[$n\]$/,/^\[/p" "$file"); then
			kwriteconfig6 --file kwinrulesrc --group "$n" --key Description ""
			kwriteconfig6 --file kwinrulesrc --group "$n" --key layer ""
			kwriteconfig6 --file kwinrulesrc --group "$n" --key layerrule ""
			kwriteconfig6 --file kwinrulesrc --group "$n" --key wmclass ""
			kwriteconfig6 --file kwinrulesrc --group "$n" --key wmclassmatch ""
			kwriteconfig6 --file kwinrulesrc --group "$n" --key wmclasscomplete ""
			# Remove the now-empty group so the rule really is gone.
			sed -i "/^\[$n\]$/,/^$/d" "$file"
			echo "Removed rule $n."
		fi
	done
	local all
	all="$(read_rule_numbers | sort -n | paste -sd, -)"
	kwriteconfig6 --file kwinrulesrc --group General --key rules "$all"
	kwriteconfig6 --file kwinrulesrc --group General --key count "$(read_rule_numbers | wc -l)"
	qdbus6 org.kde.KWin /KWin org.kde.KWin.reconfigure >/dev/null 2>&1 || true
}

status() {
	if grep -q "^Description=$DESCRIPTION$" "$(config_file)" 2>/dev/null; then
		echo "The overlay-above-fullscreen rule is installed."
	else
		echo "The overlay-above-fullscreen rule is NOT installed."
		echo "Run '$0 install' to add it."
	fi
}

case "${1:-status}" in
	install) install_rule ;;
	remove) remove_rule ;;
	status) status ;;
	*)
		echo "Usage: $0 {install|remove|status}" >&2
		exit 2
		;;
esac
