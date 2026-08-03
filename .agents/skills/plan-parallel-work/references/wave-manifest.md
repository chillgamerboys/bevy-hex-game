# Wave manifest template

Load this template only after the work has been classified as a wave.

```md
# Wave: <name>

Base: origin/dev @ <sha>
Integration owner: <owner>
Outcome: <one shippable combined outcome>
Excluded: <work explicitly outside the wave>

## Shared foundation

Live contracts:
- <contract and authority>

Required contract changes:
- <change, owner, behavior-neutral landing plan>

## Lanes

| Lane | Source branch | Owner | Base/dependency | Contracts or hot files | Focused checks |
|---|---|---|---|---|---|
| <name> | <branch> | world/gameplay/shared | <base> | <paths/contracts> | <commands/scenarios> |

## Integration order

1. <foundation>
2. <owner-local foundation>
3. <feature lanes>
4. <composition/adapters>

## Combined acceptance

- <runtime path>
- <composition or failure path>
- regeneration and return-to-title/re-entry when relevant
- affected static camera/UI/rendered-map frames to inspect, or verified-maintainer N/A
- affected video/human camera-motion/input/animation/feel route, or verified-maintainer N/A
- typed gameplay/world hooks that prove every logical claim

## Stop conditions

- unresolved behavior change across owner boundaries
- two lanes implementing the same authority differently
- source diff cannot be separated from obsolete parent state

## Cleanup

- source PR disposition
- child PR retargeting required before branch deletion
- ongoing branches that need an updated-dev reconciliation note
```
