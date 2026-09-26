#!/usr/bin/env node
// Signs what srelens hosts verify before they trust an app (#559): the root, publisher
// delegations, the catalog, and release signatures. The format is described in
// docs/extensions/trust.md and checked by crates/registry/src/extensions/trust.rs; the test
// fixtures in crates/registry/tests/fixtures/trust are this script's output.
//
// Dependency-free: node:crypto signs Ed25519, as the app release workflows' sign.mjs does.
//
//   node scripts/extensions/trust.mjs keygen <private.pem>
//   node scripts/extensions/trust.mjs key <key>
//   node scripts/extensions/trust.mjs root --version N --root <key>... --root-threshold N \
//        --catalog <key>... --catalog-threshold N --sign <key>...
//   node scripts/extensions/trust.mjs publisher --id ID --name NAME --key <key>... \
//        --namespace NS... [--version N] --sign <key>...
//   node scripts/extensions/trust.mjs bundle <publisher.json>...
//   node scripts/extensions/trust.mjs catalog --in <catalog.json> --publisher <publisher.json>... \
//        --version N --expires <RFC 3339 UTC> --sign <key>...
//   node scripts/extensions/trust.mjs release --sign <key> <manifest.json>
//
// A <key> is a PEM file (a PKCS#8 private key, or an SPKI public key where only the public
// half is needed), a file of the 32 raw public key bytes, or `seed:<64 hex>`: a private key
// derived from a seed, for test fixtures only, since anyone who reads the seed holds the key.
// Signed documents are written to stdout.
import { createHash, createPrivateKey, createPublicKey, generateKeyPairSync, sign } from 'node:crypto';
import { existsSync, readFileSync, writeFileSync } from 'node:fs';

const TYPES = {
  root: 'application/vnd.srelens.root+json',
  catalog: 'application/vnd.srelens.catalog+json',
  publisher: 'application/vnd.srelens.publisher+json',
};
// DER prefixes of an Ed25519 key: PKCS#8 around a 32-byte seed, SPKI around a public key.
const PKCS8_SEED_PREFIX = Buffer.from('302e020100300506032b657004220420', 'hex');
const SPKI_PREFIX = Buffer.from('302a300506032b6570032100', 'hex');

function fail(message) {
  console.error(`trust.mjs: ${message}`);
  process.exit(1);
}

/** A key argument as `{ privateKey?, publicKey }` KeyObjects. */
function loadKey(spec) {
  if (spec.startsWith('seed:')) {
    const seed = Buffer.from(spec.slice(5), 'hex');
    if (seed.length !== 32) fail(`${spec}: a seed is 64 hexadecimal characters`);
    const privateKey = createPrivateKey({ key: Buffer.concat([PKCS8_SEED_PREFIX, seed]), format: 'der', type: 'pkcs8' });
    return { privateKey, publicKey: createPublicKey(privateKey) };
  }
  const raw = readFileSync(spec);
  if (raw.subarray(0, 10).toString() === '-----BEGIN') {
    const text = raw.toString();
    if (text.includes('PRIVATE KEY')) {
      const privateKey = createPrivateKey(text);
      if (privateKey.asymmetricKeyType !== 'ed25519') fail(`${spec}: not an Ed25519 key`);
      return { privateKey, publicKey: createPublicKey(privateKey) };
    }
    const publicKey = createPublicKey(text);
    if (publicKey.asymmetricKeyType !== 'ed25519') fail(`${spec}: not an Ed25519 key`);
    return { publicKey };
  }
  if (raw.length !== 32) fail(`${spec}: neither a PEM key nor 32 raw public key bytes`);
  return { publicKey: createPublicKey({ key: Buffer.concat([SPKI_PREFIX, raw]), format: 'der', type: 'spki' }) };
}

function rawPublic(key) {
  return key.publicKey.export({ format: 'der', type: 'spki' }).subarray(SPKI_PREFIX.length);
}

/** SHA-256 of the raw public key, as the host computes it. */
function keyId(key) {
  return createHash('sha256').update(rawPublic(key)).digest('hex');
}

function keySpec(key) {
  return { keytype: 'ed25519', scheme: 'ed25519', keyval: { public: rawPublic(key).toString('hex') } };
}

/** DSSE's pre-authentication encoding: what the signature covers. */
function pae(type, body) {
  return Buffer.concat([Buffer.from(`DSSEv1 ${Buffer.byteLength(type)} ${type} ${body.length} `), body]);
}

function envelope(type, document, signers) {
  if (signers.length === 0) fail('name at least one --sign key');
  const body = Buffer.from(`${JSON.stringify(document, null, 2)}\n`);
  const message = pae(type, body);
  return {
    payloadType: type,
    payload: body.toString('base64'),
    signatures: signers.map((spec) => {
      const key = loadKey(spec);
      if (!key.privateKey) fail(`${spec}: signing needs the private key`);
      return { keyid: keyId(key), sig: sign(null, message, key.privateKey).toString('base64') };
    }),
  };
}

function print(value) {
  process.stdout.write(`${JSON.stringify(value, null, 2)}\n`);
}

/** `--name value` pairs, repeatable, and the positional arguments. */
function parse(args) {
  const options = {};
  const positional = [];
  for (let i = 0; i < args.length; i++) {
    if (!args[i].startsWith('--')) {
      positional.push(args[i]);
      continue;
    }
    const name = args[i].slice(2);
    if (i + 1 >= args.length) fail(`--${name} needs a value`);
    (options[name] ??= []).push(args[++i]);
  }
  return { options, positional };
}

function one(options, name) {
  const values = options[name] ?? [];
  if (values.length !== 1) fail(`give --${name} exactly once`);
  return values[0];
}

function positiveInteger(options, name, fallback) {
  const text = options[name] ? one(options, name) : fallback;
  const value = Number(text);
  if (!Number.isSafeInteger(value) || value < 1) fail(`--${name} must be a whole number of at least 1`);
  return value;
}

function role(options, name) {
  const keys = (options[name] ?? []).map(loadKey);
  if (keys.length === 0) fail(`name at least one --${name} key`);
  return { keys, threshold: positiveInteger(options, `${name}-threshold`) };
}

const commands = {
  keygen({ positional: [out] }) {
    if (!out) fail('keygen <private.pem>');
    if (existsSync(out)) fail(`${out} exists; a key is never overwritten`);
    const { privateKey } = generateKeyPairSync('ed25519');
    // Never overwrites: a key that is replaced by accident cannot be recovered.
    writeFileSync(out, privateKey.export({ format: 'pem', type: 'pkcs8' }), { mode: 0o600, flag: 'wx' });
    commands.key({ positional: [out] });
  },
  key({ positional: [spec] }) {
    if (!spec) fail('key <key>');
    const key = loadKey(spec);
    print({ keyid: keyId(key), key: keySpec(key) });
  },
  root({ options }) {
    const root = role(options, 'root');
    const catalog = role(options, 'catalog');
    const keys = {};
    for (const key of [...root.keys, ...catalog.keys]) keys[keyId(key)] = keySpec(key);
    const document = {
      _type: 'root',
      version: positiveInteger(options, 'version'),
      keys,
      roles: {
        root: { keyids: root.keys.map(keyId), threshold: root.threshold },
        catalog: { keyids: catalog.keys.map(keyId), threshold: catalog.threshold },
      },
    };
    print(envelope(TYPES.root, document, options.sign ?? []));
  },
  publisher({ options }) {
    const document = {
      _type: 'publisher',
      version: positiveInteger(options, 'version', '1'),
      id: one(options, 'id'),
      name: one(options, 'name'),
      keys: (options.key ?? []).map((spec) => keySpec(loadKey(spec))),
      namespaces: options.namespace ?? [],
    };
    print(envelope(TYPES.publisher, document, options.sign ?? []));
  },
  bundle({ positional }) {
    print(positional.map((file) => JSON.parse(readFileSync(file, 'utf8'))));
  },
  catalog({ options }) {
    const source = JSON.parse(readFileSync(one(options, 'in'), 'utf8'));
    const expires = one(options, 'expires');
    if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/.test(expires) || Number.isNaN(Date.parse(expires))) {
      fail('--expires must be an RFC 3339 UTC time such as 2026-10-31T00:00:00Z');
    }
    const document = {
      _type: 'catalog',
      schemaVersion: 2,
      version: positiveInteger(options, 'version'),
      expires,
      publishers: (options.publisher ?? []).map((file) => JSON.parse(readFileSync(file, 'utf8'))),
      // An unsigned catalog of schema version 1 gives its entries unchanged.
      extensions: source.extensions ?? [],
    };
    print(envelope(TYPES.catalog, document, options.sign ?? []));
  },
  release({ options, positional: [manifest] }) {
    if (!manifest) fail('release --sign <key> <manifest.json>');
    const key = loadKey(one(options, 'sign'));
    if (!key.privateKey) fail('signing needs the private key');
    // Over the exact manifest bytes, as every signature before #559 was; only the file
    // around it changed, so it can name its key.
    const sig = sign(null, readFileSync(manifest), key.privateKey);
    print({ keyid: keyId(key), sig: sig.toString('base64') });
  },
};

const [command, ...rest] = process.argv.slice(2);
if (!commands[command]) fail(`unknown command ${command ?? '(none)'}; see the header of this file`);
commands[command](parse(rest));
