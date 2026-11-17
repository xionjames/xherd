# A tour of xherd

The screens and the CLI, all with mocked data. A short video of the same tour is on the [project page](../../README.md#what-is-xherd); the [user guide](guide.md) explains each screen.

## Screens

| | |
|---|---|
| ![Jobs: counters that filter, jobs grouped by what you must do, each with a pill per stage of its workflow and the initials of the agent that runs it, and who in the team is busy](../assets/herd.jpg) | ![Jobs by stage: one lane per workflow, a column per stage, with the same counters](../assets/stages.jpg) |
| **Jobs.** What needs you first; a pill per stage, the agent's initials below it. | **Jobs by stage.** One lane per workflow (feature, bugfix…), a column per stage. |
| ![Inbox: what waits for you on the left, the picked item with its stage pills and a Review button on the right](../assets/inbox.jpg) | ![The Board: stages and tasks of every job in Pending, In progress and Done, with TODOs](../assets/board.jpg) |
| **Inbox.** Everything that stops a job, and what xherd approved for you. | **Board.** Every stage and task of every job, and your TODOs. |
| ![Team: one card per saved agent with its runtime, live jobs and tokens today](../assets/team.jpg) | ![A workflow's stages, each with the agent that runs it and a switch to always wait for you](../assets/flows.jpg) |
| **Team.** Saved agents: who they are, what they run on and how busy they are. | **Flows.** Give each stage of a workflow to an agent. |
| ![A job's live terminal: it works on a task, runs the tests and reports to xherd](../assets/terminal.jpg) | ![The spec of a ticket: problem, numbered requirements and how each is checked](../assets/spec.jpg) |
| **Terminal.** Each job's live session, in the browser. | **Spec.** Problem, requirements and acceptance, written by the agent. |
| ![A job waiting at the design gate: the your-call band with Approve and Request changes, and the design document](../assets/review-design.jpg) | ![A design document with a flowchart of the coupon check](../assets/design.jpg) |
| **Review a document.** Approve it, or ask for changes. | **Design.** Diagrams in the documents are drawn ([Mermaid](https://mermaid.js.org)). |
| ![A commit waiting for approval: its message, the tasks it closes, and its diff](../assets/review-commit.jpg) | ![A comment on a line of the diff, and a Send 1 comment button to send it to the job](../assets/review-comments.jpg) |
| **Review a commit.** xherd commits it when you approve. | **Comment on lines.** Comments go back to the job as one change request. |
| ![The pull request of a job watching it: state, passing checks, no conflicts, and its reviewers](../assets/pull-request.jpg) | ![A pull request with a merge conflict: a Resolve conflicts button beside Check now](../assets/resolve-conflicts.jpg) |
| **Pull request.** Checks, conflicts and reviewers, polled while the job watches it. | **Resolve conflicts.** The job merges the base branch; the merge commit always waits for you. |

### From the terminal

![The xherd CLI: list jobs, start one for a ticket, read a proposed commit's diff, approve it, ask for changes, pause a job](../assets/cli.gif)
*The everyday actions are also commands: `xherd ls`, `new`, `diff`, `approve`, `changes`, `pause` (the outputs are real, from a throwaway daemon). See the [CLI reference](../reference/cli.md).*
