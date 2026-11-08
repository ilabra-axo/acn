# Spec 0005: Spatial payloads (draft)

Goal: **the same structured message renders in 2D chat and in AR.** Spatial is an optional
block inside a contract, not a separate message type.

## `spatial` block
Defined in `nmesh.location/v1` (`$defs.spatial`). Other contracts may embed it by `$ref`.
```json
{
  "anchor": { "type": "geo", "lat": 59.91, "lng": 10.75, "alt_m": 2, "altitude_mode": "terrain", "heading_deg": 90 },
  "model":  { "url": "https://blossom.example/<sha256>", "sha256": "<sha256>", "format": "glb", "scale": 1.0 },
  "panel":  { "...": "optional nested structured payload shown as a 2D panel at the anchor" }
}
```
Anchor types:
- `geo`: ARCore Geospatial API (WGS84, terrain or rooftop anchors). Doesn't expire; preferred.
- `cloud`: ARCore Cloud Anchor ID with `expires_at`. Must be refreshed by publishing a new
  message that references the old one.

## Rendering rules
- 2D hosts: render as a map pin plus card/panel. They ignore `model`.
- AR hosts: place an anchor, load `model` (verify the sha256) or render `panel` as a Tier 0
  panel.
- Interactions (select, move, place) are emitted as Actions with the standard action contract,
  so agents handle AR input the same way as button presses.

## Privacy
Precise locations are sensitive. Spatial messages SHOULD only be sent inside E2E conversations
(NIP-17 or Marmot), never as public events.
