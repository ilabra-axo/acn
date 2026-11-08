# Spike: does Cashu reveal the preimage to the payer?

CR-002 §4.1 (Phase B) · Nov 8, 2026 · result: **yes, on any mint that supports NUT-07 and NUT-14.**

## Question

When a performer redeems a hash-locked Cashu token with the preimage, can the payer reliably learn
that preimage afterwards, without relying on the performer to publish it? The `contingent`
settlement trigger needs this: the performer's key `k` is the preimage, and the requester must get
it by paying.

## What the specs say

- **NUT-14** (HTLCs), as published on `cashubtc/nuts` main, checked 2026-11-08:
  - "The `preimage` used to spend a locked Proof can be retrieved using [NUT-07]."
  - "Caution: applications that rely on being able to retrieve the witness independently of the
    spender must check, via the mint's [info] endpoint, that NUT-07 is supported."
  - "The witness for a spent proof can be obtained with a `Proof` state check (see [NUT-07])."
  - The witness format is `{"preimage": <hex_str>, "signatures": [...]}`.
- **NUT-07** (token state check), same date: the `POST /v1/checkstate` response carries, per `Y`,
  `"witness": <str|null>`, "the serialized witness data that was used to spend the `Proof` if the
  token has a [NUT-10] spending condition that requires a witness such as in the case of P2PK
  ([NUT-11]) or HTLCs ([NUT-14])". NUT-07 itself is **optional**; support is declared in NUT-06
  info as `"7": {"supported": true}`.

So the behaviour is specified, and conditional on one discoverable capability: NUT-07 support.
Neither NUT says how long a mint must keep the witness of a spent proof.

## Method

The test is `crates/nmesh-wallet/tests/preimage_visibility.rs`. It runs against an in-process CDK
mint by default, or against any mint given in `NMESH_SPIKE_MINT`.

1. The payer tops up (fake Lightning backend) and locks 21 sat to the performer's pubkey and
   `H = sha256(k)`, with a one-hour locktime and the payer as refund key (NUT-14 receiver and
   sender paths, as in Core.md §8.1).
2. Before redemption, the payer's state check shows `UNSPENT` and no witness.
3. The performer redeems with `k` and its signature.
4. The payer, using only its own copy of the token (it knows every proof's `Y`), calls
   `POST /v1/checkstate` with a fresh wallet, and checks that every spent proof's witness carries
   `k`.

Fresh keys and a fresh `k` each run, so state from earlier runs on a persistent mint can't
interfere.

## Results

| Mint | Version | NUT-06 info | Witness returned | Preimage = `k` | Runs |
| --- | --- | --- | --- | --- | --- |
| CDK, in-process (`nmesh-mint`) | cdk 0.18.3, SQLite, fake Lightning | 7, 11, 12, 14 supported | yes, for every spent proof | yes | default `cargo test` |
| CDK, the demo cast's persistent mint on `127.0.0.1:3338` | cdk 0.18.3, SQLite | 7, 11, 12, 14 supported | yes | yes | `NMESH_SPIKE_MINT=http://127.0.0.1:3338` |
| Nutshell, local | Nutshell 0.20.2, SQLite, `FakeWallet` | 7, 8, 9, 10, 11, 12, 14, 17, 20, 29 supported | yes, for every spent proof | yes | `NMESH_SPIKE_MINT=http://127.0.0.1:3340` |

The token in each run split into three proofs; all three returned the same preimage.

Request (Nutshell; one `Y` shown):

```json
{ "Ys": ["022b2b0984b4be598d7fd75c99dab78aad2286b0c24aa1b9169905c0fccd7ec786"] }
```

Response:

```json
{
  "states": [
    {
      "Y": "022b2b0984b4be598d7fd75c99dab78aad2286b0c24aa1b9169905c0fccd7ec786",
      "state": "SPENT",
      "witness": "{\"preimage\":\"565044c24c37bf0cc5c2ad88b6095ab198b544377dbd8d59cf16e243d25e592e\",\"signatures\":[\"f2c84cedc4f0447e6474824c77b851efce24ffe22943907f44165f1e5ebc135cecc0eb43cad383da1cb108ee45793862890e321878725196b57e878a894073ea\"]}"
    }
  ]
}
```

That run's `k` was `565044c2…e592e`. The CDK response has the same shape (keys `Y`, `state`,
`witness`, with the witness as a JSON string).

To reproduce Nutshell:

```sh
# Nutshell 0.20.2 needs older marshmallow and limits than pip resolves today.
uv venv --python 3.12 .tools/nutshell && uv pip install --python .tools/nutshell/bin/python cashu "marshmallow<4" "limits<4"
MINT_BACKEND_BOLT11_SAT=FakeWallet MINT_LISTEN_PORT=3340 MINT_PRIVATE_KEY=test MINT_DATABASE=data/mint .tools/nutshell/bin/mint
NMESH_SPIKE_MINT=http://127.0.0.1:3340 cargo test -p nmesh-wallet --test preimage_visibility -- --nocapture
```

## Mint configuration that changes the result

- **NUT-07 disabled or absent.** Then there is no state check and no witness. It's discoverable
  in NUT-06 info, as NUT-14 tells applications to check.
- **Witness size limits.** Both implementations cap witnesses at 1,024 characters by default
  (Nutshell `MINT_MAX_WITNESS_LENGTH`, verified at spend time; CDK's Postgres backend clears
  stored witnesses over 1,024). A single-signature HTLC witness is about 200 characters, so ACN's
  lock (one performer key, Core.md §8.1) is well inside; large multisig witnesses could hit it.
- **Retention.** Neither NUT says how long a mint keeps spent-proof witnesses, and neither
  implementation prunes them by default. A payer should check soon after funding, or subscribe:
  Nutshell advertises NUT-17 `proof_state` subscriptions, CDK 0.18 lists none.
- **Trust.** A mint can always lie or withhold, as with every custodial Cashu operation (Core.md
  §8.6). Not new to contingent settlement.

## Decision

This is the middle case of CR-002's decision rule (implementation-dependent), but with a standard
way to discover it:

- Cashu gains `reveals_preimage_to_payer` **for any mint whose NUT-06 info declares NUT-07 and
  NUT-14 supported.** No ACN-specific declaration is needed to discover it; service
  advertisements still list the capability per mint (Core.md §8.6), so bindings can filter on it
  before fetching anything.
- As agreed for Phase A, **bindings** check it (the requester before funding, the performer before
  promising), from the mint's NUT-06 info. The validator doesn't, since that's I/O.
- `contingent` is allowed on Cashu with such mints. No Lightning is needed (D-25 stays open).
- The requester's binding must keep the `Y`s of its funding proofs and check their state promptly
  after the performer's claim (or subscribe via NUT-17 where offered). The performer's `CLAIM`
  act still publishes `k` (CR-002 §2.3); the mint path is what makes withholding it pointless.

Recorded as D-26 in Core.md §14.
