#!/bin/bash
# Installs the Ubuntu PAM stacks under ROOT/etc/pam.d and points the Arch includes at them:
#
#     packaging/deb/pam.sh ROOT
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
pam=$1/etc/pam.d

# Arch's system-auth becomes Ubuntu's common-* stacks, system-(local-)login reprise-system-login.
# auth goes to reprise-password-auth: common-auth with its fingerprint profile on waits for a finger.
install -Dm644 -t "$pam" "$here/pam.d/reprise-system-login" "$here/pam.d/reprise-password-auth"
sed -Ei '/^#/!{
  s/^(-?auth\s+)include(\s+)system-auth\b/\1include\2reprise-password-auth/
  s/^(-?)(account|password|session)(\s+)include(\s+)system-auth\b/\1\2\3include\4common-\2/
  s/include(\s+)system-(local-)?login\b/include\1reprise-system-login/
}' "$pam"/*
if grep -nE '^[^#]*include\s+(system-|common-auth\b)' "$pam"/*; then
  echo "pam.sh: a PAM file still includes an Arch stack or common-auth" >&2
  exit 1
fi
