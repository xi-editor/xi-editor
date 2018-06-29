#!/bin/bash


GIT_ROOT="$(git rev-parse --show-toplevel)"

# Checks if rustfmt is installed
check_rustfmt() {
    which rustfmt > /dev/null
    exit_status=$?
	if [ $exit_status -eq 1 ]; then
    	echo "Rustfmt is needed to contribute to xi-editor"
		exit 1
    fi
}

fmt() {
    cargo fmt -- "$1"
    exit_status=$?
	if [ $exit_status -eq 1 ]; then
    	echo "rustfmt failed to format"
        exit 1
	fi
}



main () {
	check_rustfmt
    
    local check_opt='--write-mode=check'
    
    if [[ "$TRAVIS_RUST_VERSION" == "nightly" ]]; then
        check_opt='--check'
    fi


    for i in $GIT_ROOT/rust/*; do
        if [[ $i == "$GIT_ROOT/rust/core-lib" ]]; then 
            FILES=$(find "$i/src/" -name "*rs")
            for file in ${FILES[@]}; do  # loop all items within core-lib/src/ 
                if [ -d $file ]; then

                for f in $file/*; do  # loop through sub sub dirs
                    rustfmt "$f" "$check_opt"
                done
                    
                fi 
                rustfmt "$file" "$check_opt"
            done
            continue
        fi

        if [[ $i = "$GIT_ROOT/rust/experimental" ]]; then 
            cd "$i/lang"
            fmt "$check_opt"
            cd ../../
            continue
        fi
        if [[ $i == "$GIT_ROOT/rust/src" ]]; then 
            for f in $i/*; do  # loop through sub sub dirs
                rustfmt "$f" "$check_opt"
            done
            continue
        fi
        if [ -d $i ]; then
            cd "$i"
            fmt "$check_opt"
            cd ..
        fi 
    done
}

main