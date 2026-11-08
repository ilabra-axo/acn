#!/bin/sh
# Downloads protoc (needed to build the CDK mint) into .tools/protoc. No sudo required.
set -e
cd "$(dirname "$0")/.."
V=29.3
mkdir -p .tools && cd .tools
curl -sSL -o protoc.zip "https://github.com/protocolbuffers/protobuf/releases/download/v$V/protoc-$V-linux-x86_64.zip"
python3 -c "import zipfile; zipfile.ZipFile('protoc.zip').extractall('protoc')"
chmod +x protoc/bin/protoc
rm protoc.zip
./protoc/bin/protoc --version
