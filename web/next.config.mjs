/** @type {import('next').NextConfig} */
const nextConfig = {
  // A fully static build, served by the Rust Worker. No server runtime means
  // no cold start for the shell, and the same bytes are what CI deploys and
  // what judges download.
  output: 'export',
  images: { unoptimized: true },
  // The API lives on the same origin, so no rewrites are needed — requests to
  // /api/* are handled by the Worker and everything else by this static build.
  trailingSlash: true,
};

export default nextConfig;
