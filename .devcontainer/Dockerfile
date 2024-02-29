FROM mcr.microsoft.com/vscode/devcontainers/typescript-node:20

RUN apt-get update \
  && apt-get install -y \
    build-essential \
    curl \
    libfontconfig1-dev \
    libgraphite2-dev \
    libharfbuzz-dev \
    libicu-dev \
    libssl-dev \
    yarnpkg \
    zlib1g-dev \
  && rm -rf /var/lib/apt/lists/*

USER node
RUN curl https://sh.rustup.rs -sSf |bash -s -- -y
ENV PATH="$HOME/.cargo/bin:$PATH"

CMD [ "sleep", "infinity" ]
