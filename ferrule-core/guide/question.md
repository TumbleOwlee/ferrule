# question — Relaying an agent's decision batch

## Relayed question

An agent's `status=question` names `{run_dir}/question.md` (shape: `ferrule guide author --section 'Questions'`): `ferrule prompt render {slug} question --where <gate or s<n>> --agent <agent>` prints it whole as the prompt, never in the viewer. Answers go back to the agent that asked (`ferrule guide principles --section 'Principles — implementation'`) in one message, numbered. An implementer's answered design-grade decision is appended by the implementer to `decisions.md` as `(user) [design]`; a behavioral answer lands as a spec change with its `spec-delta`; the next card move's draft render shows either. A decision marked `Assumes: <n> = <option>` is void when the user answered `<n>` otherwise; the agent asks it again in its next `question.md`, as it does a decision that depended on an answer and waited. A question without a real `Why you:` is sent back: the agent decides it and records `(chosen)`.
