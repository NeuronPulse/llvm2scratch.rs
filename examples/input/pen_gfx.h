#ifndef PEN_GFX_H
#define PEN_GFX_H

#ifdef __cplusplus
extern "C" {
#endif

/*
 * pen_gfx.h - A pixel-oriented drawing library for llvm2scratch.
 *
 * Architecture:
 *   Mechanism layer (Rust compiler backend): recognizes a handful of extern
 *   symbols and emits the corresponding Scratch native motion/pen blocks.
 *   Policy layer (this header): builds slightly higher-level primitives on top
 *   of those mechanisms.  The atomic drawing unit is a single pixel; everything
 *   else is composed from pixels.
 */

/* -------------------------------------------------------------------------- */
/* Mechanism-layer primitives (lowered to Scratch native blocks by Rust)      */
/* -------------------------------------------------------------------------- */

/* Move the (blank) sprite to (x, y). When the pen is down, the browser draws
 * a straight line between the previous position and the new one. */
void pen_goto(double x, double y);

/* Pen state. */
void pen_down(void);
void pen_up(void);
void pen_clear(void);

/* Pen appearance.  pen_color accepts Scratch hex color strings (e.g. "#ff0000").
 * pen_color_num accepts a 24-bit RGB numeric value (0xRRGGBB). */
void pen_color(const char *color);
void pen_color_num(double color);
void pen_size(double size);

/* -------------------------------------------------------------------------- */
/* Minimal math helpers (no libc dependency)                                  */
/* -------------------------------------------------------------------------- */

static inline double pg_abs(double x) {
    return x < 0.0 ? -x : x;
}

static inline double pg_min(double a, double b) {
    return a < b ? a : b;
}

static inline double pg_max(double a, double b) {
    return a > b ? a : b;
}

/* -------------------------------------------------------------------------- */
/* Pixel-oriented policy layer                                                */
/* -------------------------------------------------------------------------- */

static inline void pg_init(void) {
    pen_up();
    pen_goto(0.0, 0.0);
    pen_clear();
    pen_size(1.0);
}

/* Draw a single pixel at (x, y) in the given color.
 * Scratch's pen only draws while the sprite moves, so we make a tiny 1-unit
 * stroke. */
static inline void pg_pixel(double x, double y, const char *color) {
    pen_color(color);
    pen_up();
    pen_goto(x, y);
    pen_down();
    pen_goto(x + 0.5, y);
    pen_up();
}

/* Same as pg_pixel, but takes a 24-bit RGB numeric color (0xRRGGBB). */
static inline void pg_pixel_num(double x, double y, double color) {
    pen_color_num(color);
    pen_up();
    pen_goto(x, y);
    pen_down();
    pen_goto(x + 0.5, y);
    pen_up();
}

/* Bresenham's line algorithm drawn pixel-by-pixel. */
static inline void pg_line(double x1, double y1, double x2, double y2,
                           const char *color) {
    double dx = pg_abs(x2 - x1);
    double dy = pg_abs(y2 - y1);
    double sx = (x1 < x2) ? 1.0 : -1.0;
    double sy = (y1 < y2) ? 1.0 : -1.0;
    double err = dx - dy;

    /* Guard against degenerate lines. */
    if (dx == 0.0 && dy == 0.0) {
        pg_pixel(x1, y1, color);
        return;
    }

    double dist = dx > dy ? dx : dy;
    int steps = (int)(dist + 1.0);
    for (int i = 0; i <= steps; i += 1) {
        pg_pixel(x1, y1, color);
        if (x1 == x2 && y1 == y2) {
            break;
        }
        double e2 = 2.0 * err;
        if (e2 > -dy) {
            err -= dy;
            x1 += sx;
        }
        if (e2 < dx) {
            err += dx;
            y1 += sy;
        }
    }
}

/* Same as pg_line, but takes a 24-bit RGB numeric color (0xRRGGBB). */
static inline void pg_line_num(double x1, double y1, double x2, double y2,
                               double color) {
    double dx = pg_abs(x2 - x1);
    double dy = pg_abs(y2 - y1);
    double sx = (x1 < x2) ? 1.0 : -1.0;
    double sy = (y1 < y2) ? 1.0 : -1.0;
    double err = dx - dy;

    /* Guard against degenerate lines. */
    if (dx == 0.0 && dy == 0.0) {
        pg_pixel_num(x1, y1, color);
        return;
    }

    double dist = dx > dy ? dx : dy;
    int steps = (int)(dist + 1.0);
    for (int i = 0; i <= steps; i += 1) {
        pg_pixel_num(x1, y1, color);
        if (x1 == x2 && y1 == y2) {
            break;
        }
        double e2 = 2.0 * err;
        if (e2 > -dy) {
            err -= dy;
            x1 += sx;
        }
        if (e2 < dx) {
            err += dx;
            y1 += sy;
        }
    }
}

/* Draw a horizontal line at y from x1 to x2 with a given thickness.
 * Used by the low-resolution triangle filler to draw thick bands. */
static inline void pg_hline_num(double x1, double y, double x2, double thickness,
                                double color) {
    pen_color_num(color);
    pen_size(thickness);
    pen_up();
    pen_goto(x1, y);
    pen_down();
    pen_goto(x2, y);
    pen_up();
}

/* Fill a triangle with a flat 24-bit RGB numeric color.
 * step controls the vertical resolution: step=1 is pixel-perfect,
 * step=2 draws 2-pixel tall bands, etc.  Larger steps are much faster in
 * Scratch but produce a coarser, "low-res" look. */
static inline void pg_fill_triangle(double x1, double y1, double x2, double y2,
                                    double x3, double y3, double color,
                                    double step) {
    /* Sort vertices by y. */
    if (y1 > y2) {
        double tx = x1; x1 = x2; x2 = tx;
        double ty = y1; y1 = y2; y2 = ty;
    }
    if (y1 > y3) {
        double tx = x1; x1 = x3; x3 = tx;
        double ty = y1; y1 = y3; y3 = ty;
    }
    if (y2 > y3) {
        double tx = x2; x2 = x3; x3 = tx;
        double ty = y2; y2 = y3; y3 = ty;
    }

    double dy_top = y2 - y1;
    double dy_full = y3 - y1;
    double dy_bot = y3 - y2;

    /* Top half: y1 -> y2. */
    for (double y = y1; y < y2; y += step) {
        if (dy_top == 0.0) {
            break;
        }
        double x_left = x1 + (x2 - x1) * (y - y1) / dy_top;
        double x_right = x1 + (x3 - x1) * (y - y1) / dy_full;
        pg_hline_num(pg_min(x_left, x_right), y, pg_max(x_left, x_right), step, color);
    }

    /* Bottom half: y2 -> y3. */
    for (double y = y2; y <= y3; y += step) {
        if (dy_bot == 0.0) {
            break;
        }
        double x_left = x2 + (x3 - x2) * (y - y2) / dy_bot;
        double x_right = x1 + (x3 - x1) * (y - y1) / dy_full;
        pg_hline_num(pg_min(x_left, x_right), y, pg_max(x_left, x_right), step, color);
    }
}

#ifdef __cplusplus
}
#endif

#endif /* PEN_GFX_H */
