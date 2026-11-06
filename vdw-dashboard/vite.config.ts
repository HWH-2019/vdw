import { defineConfig } from 'vite';

export default defineConfig({
  esbuild: { jsx: 'automatic' },
  build: {
    outDir: '../vdw/dashboard-assets',
    emptyOutDir: true,
    sourcemap: false,
    rollupOptions: {
      onwarn(warning, warn) {
        if (warning.code === 'MODULE_LEVEL_DIRECTIVE' && warning.message.includes('use client')) return;
        warn(warning);
      },
    },
  },
  server: {
    // For development only. Set the URL printed by `vdw dashboard --no-open`.
    proxy: process.env.VDW_DASHBOARD_API ? {
      '/api': {
        target: process.env.VDW_DASHBOARD_API,
        changeOrigin: true,
        configure(proxy) {
          proxy.on('proxyReq', request => {
            request.setHeader('Origin', new URL(process.env.VDW_DASHBOARD_API!).origin);
          });
        },
      },
    } : undefined,
  },
});
