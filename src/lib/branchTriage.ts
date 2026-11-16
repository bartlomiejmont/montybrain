import type { MontyBrainApi } from "../api/api";

/** Keep a successfully created topic when linking fails, so retry never creates a duplicate. */
export async function createAndLinkBranch(
  api: Pick<MontyBrainApi, "createTopic" | "linkBranch">,
  project: string,
  repo: string,
  branch: string,
  title: string,
  createdTopic: string | null,
  onCreated: (slug: string) => void,
) {
  let topic = createdTopic;
  if (!topic) {
    const created = await api.createTopic(project, { title, repo, status: "planned" });
    topic = created.slug;
    onCreated(topic);
  }
  const drift = await api.linkBranch(project, repo, branch, topic);
  return { topic, drift };
}
