FROM node:22-alpine
WORKDIR /app

COPY install.sh install.ps1 ./
COPY docs/ ./docs/
COPY media/ ./media/
COPY scripts/build-public-docs-site.mjs scripts/serve-docs-site.mjs ./scripts/

RUN node scripts/build-public-docs-site.mjs dist/public-docs-site

CMD ["node", "scripts/serve-docs-site.mjs", "dist/public-docs-site"]
