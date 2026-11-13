//! An interface no nori links, as a plugin built with a newer SDK would import one.

wit_bindgen::generate!({
    inline: "package nori:plugin@5.1.0;
        interface made-up { ping: func(); }
        world unlinked { import made-up; }",
});
