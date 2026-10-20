import { accountDemo } from "./account.js";
import type { DemoOptions, DemoResult, Engine } from "./common.js";
import { poolDemo } from "./pool.js";
import { routerDemo } from "./router.js";
import { tokenDemo } from "./token.js";

export * from "./common.js";
export { accountDemo } from "./account.js";
export { impliedFeeBps, poolDemo } from "./pool.js";
export { PUMPSWAP_POOL, routerDemo, routerFixturesAvailable } from "./router.js";
export { tokenDemo } from "./token.js";

export const DEMOS: Record<Engine, (o: DemoOptions) => Promise<DemoResult>> = {
  token: tokenDemo,
  router: routerDemo,
  pool: poolDemo,
  account: accountDemo,
};
