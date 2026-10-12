import type { GetStaticPaths } from "astro";
import { PRODUCTS } from "../../../lib/changelog";
import { changelogFeed } from "../../../lib/changelog-feed";

export const prerender = true;
export const getStaticPaths = (() =>
  PRODUCTS.map((p) => ({ params: { product: p.id } }))) satisfies GetStaticPaths;
export const GET = ({ params }: { params: { product: string } }) => changelogFeed(params.product);
