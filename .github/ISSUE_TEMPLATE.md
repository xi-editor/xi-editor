# Xi-Editor Issue Template
- [ ] I have searched existing issues and could not find my issue.
- [ ] I have studied the documentation.

<!---
Please ensure the issue meets these requirements. If you are not sure, questions
are welcome on the #xi-editor channel on https://xi.zulipchat.com.
--->
## Details

_If your issue is a build or runtime error, please include the following:_
- OS / platform (e.g. macOS 10.13.2)
- rust compiler version (`rustc --version`) (we test against the latest
  stable release)
- rust compiler installation method (e.g. rustup, homebrew, source)
- the frontend you're using, if applicable
- the commit you're on, if building from source (`#3a2405b`)
- a full backtrace or error message, if available

## Expected vs Actual
When describing an issue, it is very helpful to first describe expected behavior, followed by the actual functionality. See the following example:

_Note that backticks can be used to escape code both inline and in blocks._


    Expected `xi-core -v` to provide version.
    ```
    $ xi-core -v
    xi-core 0.2.0
    ```

    Actual: xi-core skips printing out the version and starts taking input
    ```
    $ xi-core -v
      <-------- xi-core waits on input there
    ```
