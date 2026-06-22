declare module "node:test" {
  const test: (name: string, fn: () => void | Promise<void>) => void;
  export default test;
}

declare module "node:assert/strict" {
  const assert: {
    equal: (actual: unknown, expected: unknown, message?: string) => void;
    deepEqual: (actual: unknown, expected: unknown, message?: string) => void;
    ok: (value: unknown, message?: string) => void;
  };
  export default assert;
}
