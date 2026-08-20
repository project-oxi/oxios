# Task 4 — fix round 1

## Status
Complete.

## Commit
- `fix(kernel): injective fingerprint rights token`

## Changes — `crates/oxios-kernel/src/capability/resolver.rs`

1. **`rights_token` made bit-injective.** The old implementation delegated
   to `Rights::Display`, which is **not** injective on the underlying
   `u8` — `Rights(0x01)` and `Rights(0x11)` both render as `"R"`. The
   fingerprint was therefore vulnerable to silent collisions whenever
   the kernel ever carried a rights pattern whose high bits were set
   but whose low bits matched a named flag. Replaced with
   `format!("{:02x}", r.0)`, reading the public `u8` field of the tuple
   struct (`types.rs:92`).

2. **Doc comment corrected.** The previous comment claimed `Rights`
   "does not expose a public `.bits()` accessor". That was wrong — the
   field is `pub u8`, so the accessor *is* the field. Reworded to
   explain why hex encoding is necessary (bit-injectivity), to note the
   exact bug it fixes (`Display` collapses `Rights(0x01)` and
   `Rights(0x11)` to `"R"`), and to note the zero-padded width ensures
   leading-zero patterns sort deterministically.

3. **Test `rights_token_is_bit_injective_and_fingerprint_sensitive`
   added.**
   - Unit assertions on `rights_token` directly:
     * `Rights(0x01) != Rights(0x11)` (proves bit-injectivity for the
       specific pattern `Display` collapsed).
     * `Rights(0x01) -> "01"`, `Rights(0x11) -> "11"` (proves hex
       shape).
     * `READ | WRITE -> "03"`, `READ | EXECUTE -> "05"` (proves named
       combos round-trip cleanly through hex).
   - End-to-end fingerprint sensitivity: two profiles that resolve to
     the same resource (`ResourceRef::Fs`) but select tools with
     different required-rights bits (`kernel.fs.read` → `READ = 0x01`
     vs `kernel.fs.write` → `WRITE = 0x02`) yield different
     fingerprints. This covers the "R|W vs R|X" named-pair fallback
     the brief mentions, since the catalog tools in scope differ in
     exactly one named-rights bit. (Initial attempt to use
     `Rights(0x01)` vs `Rights(0x11)` directly in the bounding CSpace
     was discarded: the fingerprint captures *effective* rights
     derived from tool requirements, not the bounding cap, so varying
     the bounding alone does not flip the fingerprint. Bit-injectivity
     of the token itself is therefore proved by the unit assertions
     and the end-to-end piece proves the fingerprint actually
     propagates the token.)

## Changes — `crates/oxios-kernel/src/capability/mod.rs`

4. **Duplicated `resolver` row removed from the module layout table.**
   The table at lines 16–23 of the previous file listed `resolver`
   twice (line 19 and a duplicated row at line 24). The duplicate has
   been deleted; the `profile` row at line 18 was already intact and
   was preserved.

## Verification
- `cargo nextest run -p oxios-kernel resolver::` — 12/12 pass.
  Includes both the new test
  (`rights_token_is_bit_injective_and_fingerprint_sensitive`) and the
  pre-existing `fingerprint_is_stable_and_input_sensitive` (which was
  accidentally truncated during initial edits and then restored to
  its full body).
- `cargo check -p oxios-kernel --all-features` — clean (no warnings
  introduced; one pre-existing `let mut p2` unused-mut warning in
  `fingerprint_is_stable_and_input_sensitive` is unrelated to this
  fix and was not introduced by it).

## Tests
- `capability::resolver::tests::rights_token_is_bit_injective_and_fingerprint_sensitive` — pass.
- `capability::resolver::tests::fingerprint_is_stable_and_input_sensitive` — pass.