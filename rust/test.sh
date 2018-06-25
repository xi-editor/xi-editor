#!/bin/sh

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
    cargo fmt 
    exit_status=$?
	if [ $exit_status -eq 1 ]; then
    	echo "rustfmt failed to format"
        exit 1
	fi
}




main () {
	check_rustfmt
    
for i in $GIT_ROOT/rust/*; do
    if [[ $i == "$GIT_ROOT/rust/core-lib" ]]; then 
        FILES=$(find "$i/src/" -name "*rs")
        for file in ${FILES[@]}; do  # loop all items within core-lib/src/ 
            if [ -d $file ]; then

            for f in $file/*; do  # loop through sub sub dirs
                rustfmt "$f"
            done
                
            fi 
            rustfmt "$file"
        done
        continue
    fi

    if [[ $i == "$GIT_ROOT/rust/experimental" ]]; then 
        cd "$i/lang"
        fmt 
        cd ../../
        continue
    fi
    if [[ $i == "$GIT_ROOT/rust/src" ]]; then 
        for f in $i/*; do  # loop through sub sub dirs
            rustfmt "$f"
        done
        continue
    fi
    if [ -d $i ]; then
        cd "$i"
        fmt 
        cd ..
    fi 
        
    done
}

main