# SSH Remote Access Guide: MacBook -> Muscle

**Objective:** Control the Workstation ("Muscle") from the MacBook via Tailscale.

## 1. Verify Tailscale

Ensure Tailscale is connected on both devices.

- **Muscle IP:** `100.125.225.116`

## 2. Configure SSH Config

On your MacBook, open your config file:

```bash
nano ~/.ssh/config
```

Paste the following block at the bottom:

```text
# Heiwa Workstation (Muscle)
Host d-money
    HostName 100.125.225.116
    User devon
    # IdentityFile ~/.ssh/id_rsa  <-- Uncomment if using keys (Recommended)
```

_> **Note:** Change `User devon` if your Linux username on Muscle is different._

## 3. Connect

Run the alias from your terminal:

```bash
ssh d-money
```

## 4. (Optional) Key-Based Auth

To skip password entry:

1. **Generate Key (MacBook):** `ssh-keygen -t ed25519 -C "macbook-remote"`
2. **Copy to Muscle:** `ssh-copy-id d-money`
