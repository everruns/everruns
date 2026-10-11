import { changelogFeed } from "../../lib/changelog-feed";

export const prerender = true;
export const GET = () => changelogFeed();
