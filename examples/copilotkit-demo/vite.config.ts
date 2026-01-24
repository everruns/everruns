import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      // Proxy API requests to Everruns backend
      '/v1': {
        target: 'http://localhost:9000',
        changeOrigin: true,
      },
    },
  },
})
