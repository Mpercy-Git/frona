import nextConfig from "eslint-config-next";

export default [
  ...nextConfig,
  {
    rules: {
      // Static export — next/image optimization is unavailable at runtime
      "@next/next/no-img-element": "off",
      // New aggressive React 19 / React Compiler rules — many false positives
      // on intentional patterns (lazy ref init, setState-in-effect for sync setup)
      "react-hooks/refs": "off",
      "react-hooks/set-state-in-effect": "off",
      "react-hooks/use-memo": "off",
      "react-hooks/preserve-manual-memoization": "off",
    },
  },
];
