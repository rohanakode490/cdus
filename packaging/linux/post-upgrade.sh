#!/bin/bash
# post-upgrade.sh - Restarts cdus-agent systemd user services after a package update.
# This script runs as root during system-wide package upgrades.

set -e

echo "🔄 CDUS: Running post-upgrade hook..."

# 1. Find all active user IDs (UIDs) on the system who have a systemd user instance running.
active_uids=$(loginctl list-users --no-legend | awk '{print $1}')

for uid in $active_uids; do
    # Check if the user has a running systemd user manager
    if systemctl --machine="${uid}@" --user is-system-running &>/dev/null; then
        # Check if the cdus-agent service is loaded/active for this user
        if systemctl --machine="${uid}@" --user is-active cdus-agent.service &>/dev/null; then
            echo "🔄 Restarting cdus-agent.service for user UID $uid..."
            systemctl --machine="${uid}@" --user daemon-reload || true
            systemctl --machine="${uid}@" --user restart cdus-agent.service || true
        else
            # If the service is enabled but not active, we can also perform daemon-reload
            if systemctl --machine="${uid}@" --user is-enabled cdus-agent.service &>/dev/null; then
                echo "🔄 Reloading systemd daemon for user UID $uid..."
                systemctl --machine="${uid}@" --user daemon-reload || true
            fi
        fi
    fi
done

echo "✅ CDUS: Post-upgrade hook completed."
