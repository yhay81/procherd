FROM gcr.io/oss-fuzz-base/base-builder-rust@sha256:574eb28a5d9b7cd2408a7924047d87e2485f0c7287e81fd499564d53bf37b133

COPY . $SRC/procherd
WORKDIR $SRC/procherd
COPY .clusterfuzzlite/build.sh $SRC/
