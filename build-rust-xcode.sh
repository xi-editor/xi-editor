#!/bin/bash

if [[ -f "${HOME}/.bash_profile" ]]; then
    source "${HOME}/.bash_profile"
fi

set -e

function build_target () {
    TARGET_NAME="$1"
    cd "${SRCROOT}/$2"
    if [[ ${ACTION:-build} = "build" ]]; then
        if [[ $PLATFORM_NAME = "" ]]; then
            # default for building with xcodebuild
            PLATFORM_NAME="macosx"
        fi

        if [[ $PLATFORM_NAME = "macosx" ]]; then
            RUST_TARGET_OS="darwin"
        else
            RUST_TARGET_OS="ios"
        fi

        for ARCH in $ARCHS
        do
            if [[ $(lipo -info "${BUILT_PRODUCTS_DIR}/${TARGET_NAME}" 2>&1) != *"${ARCH}"* ]]; then
                rm -f "${BUILT_PRODUCTS_DIR}/${TARGET_NAME}"
            fi
        done

        if [[ $CONFIGURATION = "Debug" ]]; then
            RUST_CONFIGURATION="debug"
            RUST_CONFIGURATION_FLAG=""
        else
            RUST_CONFIGURATION="release"
            RUST_CONFIGURATION_FLAG="--release"
        fi

        EXECUTABLES=()
        for ARCH in $ARCHS
        do
            RUST_ARCH=$ARCH
            if [[ $RUST_ARCH = "arm64" ]]; then
                RUST_ARCH="aarch64"
            fi
            cargo build $RUST_CONFIGURATION_FLAG --target "${RUST_ARCH}-apple-${RUST_TARGET_OS}"
            EXECUTABLES+=("target/${RUST_ARCH}-apple-${RUST_TARGET_OS}/${RUST_CONFIGURATION}/${TARGET_NAME}")
        done

        mkdir -p "${BUILT_PRODUCTS_DIR}"
        xcrun --sdk $PLATFORM_NAME lipo -create "${EXECUTABLES[@]}" -output "${BUILT_PRODUCTS_DIR}/${TARGET_NAME}"
    elif [[ $ACTION = "clean" ]]; then
        cargo clean
        rm -f "${BUILT_PRODUCTS_DIR}/${TARGET_NAME}"
    fi
}

build_target xi-core rust
build_target xi-syntect-plugin rust/syntect-plugin

# workaround for https://github.com/travis-ci/travis-ci/issues/6522
set +e
