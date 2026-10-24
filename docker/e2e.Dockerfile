# Firefox end-to-end tests on Linux, as CI runs them: Playwright's browsers plus the
# .NET 10 SDK and Python that the export/ILVerify tests use, and socat to forward the
# host's dev servers (Vite on 5173, the rari RSC server on 3000) to the container's
# localhost. Built by `npm run test:e2e:firefox`; analyzed assemblies are never run.
ARG PLAYWRIGHT_VERSION=1.63.0
FROM mcr.microsoft.com/playwright:v${PLAYWRIGHT_VERSION}-noble
RUN apt-get update \
 && apt-get install -y --no-install-recommends python3 curl ca-certificates libicu74 socat \
 && rm -rf /var/lib/apt/lists/* \
 && curl -sSL https://dot.net/v1/dotnet-install.sh -o /tmp/dotnet-install.sh \
 && bash /tmp/dotnet-install.sh --channel 10.0 --install-dir /usr/share/dotnet \
 && ln -s /usr/share/dotnet/dotnet /usr/local/bin/dotnet \
 && rm /tmp/dotnet-install.sh
ENV DOTNET_CLI_TELEMETRY_OPTOUT=1 DOTNET_NOLOGO=1 DOTNET_CLI_UI_LANGUAGE=en DOTNET_ROOT=/usr/share/dotnet
