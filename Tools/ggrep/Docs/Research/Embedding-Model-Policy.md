# Embedding Models + Download Policy (Phase II)

Status: Accepted (2026-01-03)
Scope: `Tools/ggrep`

## Default models (Phase II)

| Role | Model ID (pinned) | Source | License |
| --- | --- | --- | --- |
| Dense retrieval | `ibm-granite/granite-embedding-small-english-r2@c949f235cb63fcbd58b1b9e139ff63c8be764eeb` | Hugging Face | Apache-2.0 |
| ColBERT rerank | `answerdotai/answerai-colbert-small-v1@be1703c55532145a844da800eea4c9a692d7e267` | Hugging Face | Apache-2.0 |

Model license references:
- Granite embedding small English r2: https://huggingface.co/ibm-granite/granite-embedding-small-english-r2
- Answer.AI ColBERT small v1: https://huggingface.co/answerdotai/answerai-colbert-small-v1

## Artifact caches

- Models cache: `~/.ggrep/models/`
- Grammars cache: `~/.ggrep/grammars/`

## Network policy (downloads)

- Default behavior: on-demand downloads for missing models and grammars.
- Offline mode: set `GGREP_OFFLINE=1` to disable all network downloads.
  - In offline mode, ggrep will use only cached artifacts; missing models will error.
  - Use `ggrep setup` (with downloads enabled) to pre-seed caches before going offline.

## Notes

- Model IDs MUST be pinned to immutable revisions for determinism.
- Grammar URLs are pinned to versioned release assets (no `latest`).
