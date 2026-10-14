#!/bin/sh
# Builds the update-signature test fixtures with a throwaway keyring. Only
# public keys and signatures leave the container; no secret key does.
set -eu
export GNUPGHOME=$(mktemp -d)
cd /out
g() { gpg --batch --quiet --pinentry-mode loopback --passphrase '' "$@"; }
fpr_of() { gpg --batch --with-colons --list-keys "$1" | awk -F: '/^fpr/{print $10; exit}'; }
printf 'the bytes a release would sign\n' > data.txt

# A key that is not in KEYS. Made in 2020 so it can also sign "back then".
g --faked-system-time 20200101T000000 \
  --quick-gen-key "srelens test key, untrusted <untrusted@test.invalid>" ed25519 sign 0
U=$(fpr_of untrusted@test.invalid)
g --local-user "$U" --armor --detach-sign --output data.txt.untrusted.asc data.txt
g --local-user "$U" --armor --detach-sign --textmode --output data.txt.textmode.asc data.txt
g --local-user "$U" --armor --detach-sign --faked-system-time 20200601T000000 \
  --default-sig-expire 1d --output data.txt.sig-expired.asc data.txt
gpg --batch --armor --export "$U" > untrusted-key.asc

# A key that signs, and is then revoked with the certificate gpg made for it.
g --quick-gen-key "srelens test key, revoked <revoked@test.invalid>" ed25519 sign 0
R=$(fpr_of revoked@test.invalid)
g --local-user "$R" --armor --detach-sign --output data.txt.revoked.asc data.txt
sed 's/^:-----BEGIN/-----BEGIN/' "$GNUPGHOME/openpgp-revocs.d/$R.rev" > /tmp/revoke.asc
g --import /tmp/revoke.asc
gpg --batch --armor --export "$R" > revoked-key.asc

# A key made in 2020 with a one-year life, which signed while it was valid.
g --faked-system-time 20200101T000000 \
  --quick-gen-key "srelens test key, expired <expired@test.invalid>" ed25519 sign 1y
E=$(fpr_of expired@test.invalid)
g --local-user "$E" --faked-system-time 20200601T000000 --armor --detach-sign \
  --output data.txt.expired.asc data.txt
gpg --batch --armor --export "$E" > expired-key.asc

# A key made in 2020 with a one-year life, extended to never expire before the
# year was out. gpg's key editor replaces a self-signature in place, but
# importing the old and the new copy into one keyring appends instead, and
# exporting that keyring carries both self-signatures. That merged export is
# the fixture: its newest self-signature is the one that counts.
g --faked-system-time 20200101T000000 \
  --quick-gen-key "srelens test key, extended <extended@test.invalid>" ed25519 sign 1y
X=$(fpr_of extended@test.invalid)
gpg --batch --export "$X" > /tmp/extended-before.gpg
g --faked-system-time 20200601T000000 --quick-set-expire "$X" 0
gpg --batch --export "$X" > /tmp/extended-after.gpg
g --local-user "$X" --armor --detach-sign --output data.txt.extended.asc data.txt
MERGED=$(mktemp -d)
GNUPGHOME=$MERGED gpg --batch --quiet --import /tmp/extended-before.gpg
GNUPGHOME=$MERGED gpg --batch --quiet --import /tmp/extended-after.gpg
GNUPGHOME=$MERGED gpg --batch --armor --export "$X" > extended-key.asc

echo "untrusted $U"; echo "revoked   $R"; echo "expired   $E"; echo "extended  $X"
gpg --batch --list-keys --with-colons | awk -F: '/^pub/{print "pub validity=" $2 " created=" $6 " expires=" $7}'
for s in untrusted textmode sig-expired revoked expired extended; do
  printf '%-12s ' "$s"
  gpg --batch --list-packets "data.txt.$s.asc" 2>/dev/null \
    | grep -o 'sigclass 0x0[01]\|sig expires after [^)]*\|created [0-9]*' | tr '\n' ' '
  echo
done
