# /dream Input

Thread id: `{{thread_id}}`
Rollout path: `{{rollout_path}}`
Repo root: `{{repo_root}}`
Memory root: `{{memory_root}}`
Repo skill root: `{{repo_skill_root}}`
Repo AGENTS path: `{{agents_path}}`

## Existing Repo Memory

{{existing_memory}}

## Existing Repo AGENTS

{{existing_agents}}

## AGENTS Fragments Visible In This Thread

{{visible_agents_fragments}}

## Skill Fragments Visible In This Thread

{{visible_skill_fragments}}

## Repo-local Skill Candidates

Existing skill updates must target only these candidate paths. The list may include repo-local skills auto-discovered under `{{repo_skill_root}}`, even if they were not explicitly referenced in the thread.

{{skill_candidates}}

## Model-visible Rollout Items

{{rollout_items_json}}
