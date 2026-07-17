#include "pen_gfx.h"

/* -------------------------------------------------------------------------- */
/* Tiny 3x5 bitmap font for digits and decimal point                          */
/* -------------------------------------------------------------------------- */
#define CHAR_W 3
#define CHAR_H 5
#define FONT_GAP 1

static const unsigned char font[11][CHAR_H] = {
    {0b111, 0b101, 0b101, 0b101, 0b111}, /* 0 */
    {0b010, 0b110, 0b010, 0b010, 0b111}, /* 1 */
    {0b111, 0b001, 0b111, 0b100, 0b111}, /* 2 */
    {0b111, 0b001, 0b111, 0b001, 0b111}, /* 3 */
    {0b101, 0b101, 0b111, 0b001, 0b001}, /* 4 */
    {0b111, 0b100, 0b111, 0b001, 0b111}, /* 5 */
    {0b111, 0b100, 0b111, 0b101, 0b111}, /* 6 */
    {0b111, 0b001, 0b001, 0b010, 0b010}, /* 7 */
    {0b111, 0b101, 0b111, 0b101, 0b111}, /* 8 */
    {0b111, 0b101, 0b111, 0b001, 0b111}, /* 9 */
    {0b000, 0b000, 0b000, 0b000, 0b010}, /* . (decimal point) */
};

#define DIGIT_INDEX_DOT 10

/* -------------------------------------------------------------------------- */
/* Spigot algorithm for computing decimal digits of pi                        */
/* -------------------------------------------------------------------------- */
#define N_DIGITS 16
#define SPIGOT_LEN ((10 * N_DIGITS) / 3 + 2)

static int pi_digits[N_DIGITS + 1];

static void compute_pi(void) {
    int a[SPIGOT_LEN];
    for (int i = 0; i < SPIGOT_LEN; i += 1) {
        a[i] = 2;
    }

    int nines = 0;
    int predigit = 0;
    int out_idx = 0;

    for (int j = 0; j < N_DIGITS; j += 1) {
        int q = 0;
        for (int i = SPIGOT_LEN - 1; i >= 0; i -= 1) {
            int x = 10 * a[i] + q * (i + 1);
            a[i] = x % (2 * i + 1);
            q = x / (2 * i + 1);
        }
        a[0] = q % 10;
        q = q / 10;

        if (q == 9) {
            nines += 1;
        } else if (q == 10) {
            pi_digits[out_idx] = predigit + 1;
            out_idx += 1;
            for (int k = 0; k < nines; k += 1) {
                pi_digits[out_idx] = 0;
                out_idx += 1;
            }
            predigit = 0;
            nines = 0;
        } else {
            if (j > 0) {
                pi_digits[out_idx] = predigit;
                out_idx += 1;
            }
            predigit = q;
            for (int k = 0; k < nines; k += 1) {
                pi_digits[out_idx] = 9;
                out_idx += 1;
            }
            nines = 0;
        }
    }
    pi_digits[out_idx] = predigit;
}

/* -------------------------------------------------------------------------- */
/* Bitmap font rendering helpers                                              */
/* -------------------------------------------------------------------------- */
#define PIXEL_SIZE 4.0

static void render_char(int digit_index, double x, double y, double color) {
    for (int row = 0; row < CHAR_H; row += 1) {
        unsigned char row_bits = font[digit_index][row];
        for (int col = 0; col < CHAR_W; col += 1) {
            if ((row_bits >> (CHAR_W - 1 - col)) & 1u) {
                pen_color_num(color);
                pen_up();
                pen_goto(x + col * PIXEL_SIZE, y - row * PIXEL_SIZE);
                pen_down();
                pen_goto(x + col * PIXEL_SIZE + 0.5, y - row * PIXEL_SIZE);
                pen_up();
            }
        }
    }
}

static void render_pi(double start_x, double start_y, double color) {
    double x = start_x;
    double y = start_y;

    render_char(3, x, y, color);
    x += (CHAR_W + FONT_GAP) * PIXEL_SIZE;

    render_char(DIGIT_INDEX_DOT, x, y, color);
    x += (CHAR_W + FONT_GAP) * PIXEL_SIZE;

    for (int i = 1; i <= N_DIGITS; i += 1) {
        render_char(pi_digits[i], x, y, color);
        x += (CHAR_W + FONT_GAP) * PIXEL_SIZE;
    }
}

/* -------------------------------------------------------------------------- */
/* Main demo: compute pi with a spigot and draw it once                       */
/* -------------------------------------------------------------------------- */
int main(void) {
    pg_init();
    compute_pi();

    double cx = -120.0;
    double cy = 20.0;
    double color = 0x00ff00;

    pen_clear();
    render_pi(cx, cy, color);

    return 0;
}
