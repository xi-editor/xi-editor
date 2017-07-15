class LineCache(object):
    """Basic access to the core's buffer.

    LineCache behaves like a list of lines. Individual lines can be accessed
    with the lines[idx] syntax. If a line is not present in the cache,
    it will be fetched, blocking the caller until it arrives.
    """
    def __init__(self, view_id, total_bytes, peer, revision, test_data=None):
        self.total_bytes = total_bytes
        # keep ends to make calculating offsets simple
        self.revision = revision
        self.peer = peer
        self.view_id = view_id

        raw_data = test_data
        if raw_data is None:
            raw_data = peer.get_data(self.view_id, 0, self.revision)
        self.raw_lines = raw_data.splitlines(True) or ['']  # handle empty buffer
        self._recalculate_offsets()

    def _recalculate_offsets(self):
        self.offsets = [0] + [len(l) for l in self.raw_lines]
        tally = 0
        for idx in range(len(self.offsets)):
            new_tally = self.offsets[idx] + tally
            self.offsets[idx] += tally
            tally = new_tally

    def __len__(self):
        # TODO: to report our length, in lines, we have to go get all lines, which is bad
        while self.has_missing():
            self.get_data(self.peer, to_offset=self.total_bytes)
        return len(self.raw_lines)

    def __getitem__(self, idx):
        while idx >= len(self.raw_lines) and self.has_missing():
            self.get_data(self.peer, to_offset=self.total_bytes)
        # trim trailing newline
        return self.raw_lines[idx].rstrip('\n')

    def has_missing(self):
        """Returns true if the cache does not have a copy of the full buffer."""
        return self.offsets[-1] != self.total_bytes

    def linecol_for_offset(self, offset):
        """Given a byte offset, returns the corresponding line and column.

        Raises IndexError if the offset is out of bounds on the buffer.
        """
        if offset < 0 or offset > self.total_bytes:
            raise IndexError("offset {} invalid for buffer length {}".format(
                offset, self.total_bytes))
        if offset == 0:
            return (0, 0)
        if offset > self.offsets[-1]:
            self.get_data(offset, self.peer)

        line_nb = bisect.bisect_left(self.offsets, offset) - 1
        col = offset - self.offsets[line_nb]
        return (line_nb, col)

    def previous_word(self, offset):
        """Returns the word immediately preceding `offset`, or None.

        Word is defined as a "sequence of non-whitespace characters".
        If the offset is inside a word, the left half of the word will be returned.

        Returns None if no words precede `offset`.

        Raises `IndexError` if `offset` is out of bounds of the buffer.
        """

        line_nb, col = self.linecol_for_offset(offset)
        line = self[line_nb]
        word = line[:col].split()[-1]
        return word or None

    def apply_update(self, peer, author, rev, start, end, new_len, edit_type, text=None):
        # an update is bytes; can span multiple lines.
        text = text or ""
        # FIXME: this can fail on very large updates. In that case we should
        # clear raw_lines, update total_bytes, and maybe do a fetch?
        assert len(text) == new_len
        if end > self.total_bytes:
            self.get_data(peer, end)
        self.revision = rev
        self.total_bytes += (start-end) + new_len

        # cannot index past last offset (which is really the bounds of the buffer)
        first_changed_idx = bisect.bisect(self.offsets[:-1], start) - 1
        last_changed_idx = bisect.bisect(self.offsets[:-1], end) - 1

        orig_first_line = self.raw_lines[first_changed_idx]
        first_line_start = start - self.offsets[first_changed_idx]
        f_end = min(end - self.offsets[first_changed_idx], len(orig_first_line))
        first_line = ''.join((orig_first_line[:first_line_start], text, orig_first_line[f_end:]))

        # append remainder of last affected line, then recalculate lines in that interval
        if first_changed_idx != last_changed_idx:
            l_end = end - self.offsets[last_changed_idx]
            first_line += self.raw_lines[last_changed_idx][l_end:]
        new_lines = first_line.splitlines(True) or ['']

        # list[5:5] = [] is an insert without replacement
        self.offsets[first_changed_idx + 1:last_changed_idx + 2] = [0 for _ in range(len(new_lines))]
        # now update offsets:
        for idx in range(len(new_lines)):
            offset_idx = first_changed_idx + idx
            self.offsets[offset_idx + 1] = self.offsets[offset_idx] + len(new_lines[idx])

        # update all offsets after the edit
        offset_to_update = first_changed_idx + len(new_lines) + 1
        for idx in range(offset_to_update, len(self.offsets)):
            self.offsets[idx] += ((start-end) + new_len)

        self.raw_lines[first_changed_idx:last_changed_idx+1] = new_lines

    def get_data(self, peer, to_offset):
        while to_offset > self.offsets[-1]:
            raw_data = peer.get_data(self.view_id, self.offsets[-1], self.revision)
            raw_lines = raw_data.splitlines(True)

            # if current last line does not contain newline, append first new line directly
            if self.raw_lines[-1][-1] != '\n':
                self.raw_lines[-1] += raw_lines[0]
                self.offsets[-1] += len(raw_lines[0])
                raw_lines = raw_lines[1:]

            for idx in range(len(raw_lines)):
                prev_idx = len(self.offsets) - 1
                self.offsets.append(len(raw_lines[idx]) + self.offsets[prev_idx])
                self.raw_lines.extend(raw_lines)
