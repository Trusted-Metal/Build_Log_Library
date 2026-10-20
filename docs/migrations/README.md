# docs/migrations/

**Migration manifests**: one document per version transition, named `vX_to_vY.md` (for example `v16_to_v17.md`, `v17_to_v18.md`, and the reverse directions).

Each manifest holds a change table (category, source location, target location, common-data-model field, lossy or not), the rules applied, and an **API impact** section for anything a consumer can see (requirement BL-VA-07).

**None yet.** They arrive with the conversion work (IP-14), once the adapters they describe exist. A manifest states the date its facts were verified.
