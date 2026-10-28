# osifont

`osifont-subset.ttf` is a subset of `osifont-lgpl3fe.ttf` from
<https://github.com/hikikomori82/osifont>, a free font drawn to ISO 3098
(the lettering standard for technical drawings). That file is offered by
its authors under the GNU LGPL version 3 **with the GPL font exception**,
which is why it is the one embedded here: the exception says that a
document which embeds the font is not itself covered by the licence, so
the PDFs this crate writes stay the user's own.

The subset keeps the glyphs for the 193 characters listed in `glyphs.rs`
(ASCII, Latin-1 letters and symbols, and the drawing symbols ⌀ ∅ □ ∠ ⌒
⌓ – — ’ ‰ €). Nothing else about the font was changed. The subset is made
by the small tool in the repo's `tools/fonttool/` folder; run it again
after adding characters to that list.

`sRGB-v4.icc` next to it is the 480-byte sRGB profile from
<https://github.com/saucecontrol/Compact-ICC-Profiles> (CC0), used as the
PDF/A output intent.
