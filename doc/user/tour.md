# A tour of xherd

The screens and the CLI, all with mocked data. A short video of the same tour is on the [project page](../../README.md#what-is-xherd); the [user guide](guide.md) explains each screen.

## Screens

| | |
|---|---|
| ![The Herd: two jobs waiting for you on top, every job below with its status, progress and tasks](../assets/herd.jpg) | ![A job's live terminal: it starts task T02, writes files, runs the tests and reports to xherd](../assets/terminal.jpg) |
| **Herd.** What needs you on top, every job below. | **Terminal.** Each job's live session, in the browser. |
| ![The spec of a ticket: problem, numbered requirements and how each is checked](../assets/spec.jpg) | ![A job waiting at the design gate: the design document, with Approve and Request changes on top](../assets/review-design.jpg) |
| **Spec.** Problem, requirements and acceptance, written by the job. | **Review a document.** Approve it, or ask for changes. |
| ![A design document with a flowchart of the coupon check](../assets/design.jpg) | ![A design document with a data table and a sequence diagram of the payment transaction](../assets/design-diagram.jpg) |
| **Design.** Diagrams in the documents are drawn ([Mermaid](https://mermaid.js.org)). | **Design.** Data, interfaces and the tricky parts, as diagrams. |
| ![A commit waiting for approval: its diff, the commit message and the files it contains](../assets/review-commit.jpg) | ![A review comment on a line of the diff, and a Send 2 comments button to send them to the job](../assets/review-comments.jpg) |
| **Review a commit.** xherd commits it when you approve. | **Comment on lines.** Comments go back to the job as one change request. |
| ![The Board: stages and tasks of every job in Pending, In progress and Done, with TODOs](../assets/board.jpg) | ![Needs attention: jobs waiting for you, a due TODO, and an approval made by xherd](../assets/attention.jpg) |
| **Board.** Every stage and task of every job, and your TODOs. | **Needs attention.** Everything waiting on you in one place. |
| ![The pull request of a job watching it: state, passing checks, no conflicts, and its reviewers](../assets/pull-request.jpg) | |
| **Pull request.** Checks, conflicts and reviewers, polled while the job watches it. | |

### From the terminal

![The xherd CLI: list jobs, start one for a ticket, read a proposed commit's diff, approve it, ask for changes, pause a job](../assets/cli.gif)
*The everyday actions are also commands: `xherd ls`, `new`, `diff`, `approve`, `changes`, `pause` (the outputs are real, from a throwaway daemon). See the [CLI reference](../reference/cli.md).*
