# iMessage Channel Setup

This guide covers running OpenCrust as an iMessage bot on macOS.

## Prerequisites

- macOS 12 (Monterey) or later
- Apple ID signed into **Messages.app** (iMessage must be active)
- OpenCrust binary built or installed

## 1. Grant Full Disk Access

OpenCrust reads `~/Library/Messages/chat.db` to detect incoming messages. macOS requires **Full Disk Access** for any process that reads this file.

1. Open **System Settings** (or System Preferences on older macOS)
2. Navigate to **Privacy & Security > Full Disk Access**
3. Click the **+** button (you may need to unlock with your password)
4. Add the **OpenCrust binary** (`/usr/local/bin/opencrust` or wherever you installed it)
5. If running from a terminal (e.g. during development), also add your **Terminal app** (Terminal.app, iTerm2, etc.)
6. Toggle the switch **on** for each entry

> Without Full Disk Access, OpenCrust will fail at startup with an error like:
> `failed to open chat.db: unable to open database file`

## 2. Configure the iMessage channel

Add the iMessage channel to your `~/.opencrust/config.yml`:

```yaml
channels:
  imessage:
    type: imessage
    enabled: true
    settings:
      poll_interval_secs: 2  # how often to check for new messages (default: 2)
```

## 3. Run OpenCrust

### Development / foreground

```bash
opencrust daemon
```

### Production / launchd (recommended)

A launchd plist template is provided at `deploy/macos/com.opencrust.gateway.plist`.

1. Create the log directory:

   ```bash
   mkdir -p ~/Library/Logs/opencrust
   ```

2. Copy and edit the plist (update the binary path if needed):

   ```bash
   cp deploy/macos/com.opencrust.gateway.plist ~/Library/LaunchAgents/
   ```

3. Load the service:

   ```bash
   launchctl load ~/Library/LaunchAgents/com.opencrust.gateway.plist
   ```

4. Verify it's running:

   ```bash
   launchctl list | grep opencrust
   ```

5. To stop:

   ```bash
   launchctl unload ~/Library/LaunchAgents/com.opencrust.gateway.plist
   ```

## 4. Gatekeeper (unsigned binaries)

If you built OpenCrust from source or downloaded an unsigned binary, macOS Gatekeeper will block execution.

### Option A: Remove quarantine attribute

```bash
xattr -cr /usr/local/bin/opencrust
```

### Option B: Allow in System Settings

After the first blocked attempt, go to **System Settings > Privacy & Security** and click **Allow Anyway** next to the OpenCrust entry.

### Option C: Notarize for distribution

If distributing the binary to others, sign and notarize it with an Apple Developer account to avoid Gatekeeper prompts entirely. See [Apple's notarization docs](https://developer.apple.com/documentation/security/notarizing_macos_software_before_distribution).

## 5. Group chats

OpenCrust supports both direct messages and group chats:

- **Direct messages**: Routed to a session per sender (e.g. `imessage-+15551234567`)
- **Group chats**: Routed to a session per group (e.g. `imessage-chat123456789`)
- Replies to group chats are sent back to the group
- The allowlist checks the actual sender, not the group name

## 6. Troubleshooting

### "failed to open chat.db"

Full Disk Access has not been granted. See step 1.

### "failed to spawn osascript"

The `osascript` binary is missing or not in PATH. This shouldn't happen on a standard macOS install. Check that `/usr/bin/osascript` exists.

### "osascript exited with ..."

Messages.app may not be running or iMessage may not be signed in. Open Messages.app and verify your Apple ID is active.

### Messages not being detected

- Check that `poll_interval_secs` is reasonable (1-5 seconds)
- Verify `chat.db` is being updated: `sqlite3 ~/Library/Messages/chat.db "SELECT MAX(ROWID) FROM message"`
- Check OpenCrust logs: `tail -f ~/Library/Logs/opencrust/gateway.log`

### Replies not sending

- Ensure Messages.app is running (AppleScript drives it)
- For group chat replies, the group identifier must match the `cache_roomnames` value in chat.db
