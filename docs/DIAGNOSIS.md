# Diagnosis

A deterministic local rule engine. It needs no AI, no network and no storage scan (a scan only adds cleanup suggestions).

## Pipeline

`gather` (commands/diagnosis.rs) -> `Input` -> `RULES` (diagnosis/rules.rs) -> ranked `DiagnosisReport` (diagnosis/engine.rs).

`Input` = memory metrics, pressure report, workloads, growth findings, disks + disk health, optional Docker status, optional storage scan entries. Rules are pure functions `fn(&Input, &Config) -> Option<Diagnosis>`; thresholds live in `Config` (defaults from the spec).

## Rules

| Rule id | Fires when | Says |
|---------|-----------|------|
| `memory-pressure` | pressure state is MODERATE or worse | RAM %, top contributors (workloads with >= 3% of RAM, max 3), whether they dominate |
| `docker-wsl` | Docker/WSL2 > 4 GB and RAM > 85% | idle containers (CPU < 1%), WSL VM holding memory with no containers, `.wslconfig` cap |
| `android-gradle` | Gradle workload > 3 GB and pressure >= MODERATE | stop daemons, cap `org.gradle.jvmargs`. Named "Android" only with Android evidence |
| `node-dev` | Node workload > 2 GB and pressure >= MODERATE | restart dev servers (Next.js named when detected) |
| `memory-growth` | growth tracker has findings | "Possible memory growth/leak", confidence, caveat that caches grow too |
| `low-disk` | a drive is WARNING or worse, or has < 10 GB free | numbers, trend prediction, cleanup candidates on that drive |
| `cleanable-caches` | >= 2 GB of SAFE_TO_CLEAN items | info-level suggestion |
| `healthy` | nothing else fired | calm summary |

## Ranking and status

Findings sort by severity (`critical > high > warning > info`), ties by rule order above. The first is the **headline**. Status: critical/high -> `problem`, warning -> `attention`, info -> `healthy`.

## Explaining why

The summary names numbers and causes. For example (96% RAM, Gradle 4.1 GB, Docker 4.8 GB):

> RAM is at 96%. Docker/WSL2 and Android Gradle build account for approximately 8.9 GB, making them the primary contributors.

"Primary contributors" is used when the named workloads hold at least half of the **used** RAM; otherwise the text says usage is spread across many processes. If nothing identifiable is large, it says so and confidence drops to 40%.

Confidence for `memory-pressure` is `0.45 + 0.45 x coverage of used RAM (+ 0.1 if concentrated)`, capped at 95%. It is a statement about the attribution, not about whether the PC is slow.

## Recommendations

Every recommendation has `problem`, `explanation`, `expectedBenefit` (text and bytes when known), `risk` (low/medium/high), and either an `action` (an id plus parameters, executed only through the action engine) or `manualSteps`. A test asserts every recommendation produced by the rules has all of these and a way to act.

Disk recommendations follow the safety class of the scanned item: SAFE_TO_CLEAN -> low-risk action; SAFE_TO_REVIEW -> medium-risk action; REQUIRES_REVIEW -> manual review only; DO_NOT_TOUCH -> never offered. Items on other drives are not offered for a low C:.

## Adding a rule

1. Write `fn my_rule(i: &Input, c: &Config) -> Option<Diagnosis>` in `rules.rs`; put thresholds in `Config`.
2. Add it to `RULES` (order = tie-break).
3. If it recommends an action, register that action in `actions/builtin.rs` (a test checks every referenced id exists).
4. Add scenario tests in `diagnosis/engine.rs` using the `World` fixture.
