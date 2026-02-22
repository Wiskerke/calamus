# Calamus

A tool and library in Rust for parsing and rendering Supernote `.note` files.

## Goal

- Provide a CLI tool, that renders a Supernote file as svg or png
- Provide a wasm output, to display note pages on a webpage (using the bitmap rendering)

## Experimental goal

There are some more experimental goals, for which this project is a building block.

I often make notebooks with a lot of handwritten text, but once every while a diagram or drawing. I would like to be able to basically extract the notes as markdown, and then take the diagrams, drawing and keep those as basic images along with the markdown. 

I have a subscription with claude, so I can use that for OCR and creating the markdown. But claude is terrible at cropping the diagrams from the notebook.

So I was thinking that if I just draw a rectangle around the diagrams, and then use the stroke data to find that rectangle, it should be doable to extract those diagrams and handle them separately.

## Thanks and inspired by

These are some pages and projects that were used to create this project. 

Information about the paths to create the svg comes mostly from the experiments of walnut356. The hexfiles were also useful to start seeing how the note format is structured.
- [Investigating the SuperNote Notebook Format](https://walnut356.github.io/posts/inspecting-the-supernote-note-format/) by walnut356
- [snlib](https://github.com/Walnut356/snlib) (Rust) by walnut356 -- exact stroke binary layout

Supernote-tool seems to the parent of all supernote parsers, and most info how to render using the bitmap data is based on this.
- [supernote-tool](https://github.com/jya-dev/supernote-tool) (Python, Apache 2.0) by jya-dev

I found PySN-digest youtube videos inspiring. Having said that, I don't expect to move in that same direction.
- [pysn-digest](https://gitlab.com/mmujynya/pysn-digest) (Python) by mmujynya
