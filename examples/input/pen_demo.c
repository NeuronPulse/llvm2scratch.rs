#include "pen_gfx.h"

/* -------------------------------------------------------------------------- */
/* Minimal math helpers (no libc dependency)                                  */
/* -------------------------------------------------------------------------- */
/* Taylor-series approximations for sin/cos, accurate enough for a rotating
 * wireframe cube.  Argument is kept in [-pi, pi] first. */
static inline double pg_sin(double x) {
    while (x > 3.141592653589793) {
        x -= 6.283185307179586;
    }
    while (x < -3.141592653589793) {
        x += 6.283185307179586;
    }
    double x2 = x * x;
    double x3 = x2 * x;
    double x5 = x3 * x2;
    double x7 = x5 * x2;
    return x - x3 / 6.0 + x5 / 120.0 - x7 / 5040.0;
}

static inline double pg_cos(double x) {
    return pg_sin(x + 1.570796326794897);
}

/* 24-bit RGB colors used directly by the pen extension. */
#define COLOR_RED        0xff0000
#define COLOR_DARK_RED   0x800000
#define COLOR_GREEN      0x00ff00
#define COLOR_DARK_GREEN 0x008000
#define COLOR_BLUE       0x0000ff
#define COLOR_DARK_BLUE  0x000080

/* -------------------------------------------------------------------------- */
/* 3D cube definition                                                         */
/* -------------------------------------------------------------------------- */
#define N_VERTICES 8
#define N_FACES 6
#define HALF_SIZE 64.0
#define POINT_STEP 12.0

/* Vertices of a centered axis-aligned cube.  The cube is much larger than
 * before, but we render it as a sparse point cloud so the per-frame cost
 * stays acceptable inside Scratch. */
static const double vertices[N_VERTICES][3] = {
    {-HALF_SIZE, -HALF_SIZE, -HALF_SIZE}, {HALF_SIZE, -HALF_SIZE, -HALF_SIZE},
    {HALF_SIZE,  HALF_SIZE, -HALF_SIZE}, {-HALF_SIZE,  HALF_SIZE, -HALF_SIZE},
    {-HALF_SIZE, -HALF_SIZE,  HALF_SIZE}, {HALF_SIZE, -HALF_SIZE,  HALF_SIZE},
    {HALF_SIZE,  HALF_SIZE,  HALF_SIZE}, {-HALF_SIZE,  HALF_SIZE,  HALF_SIZE}
};

/* Faces: {v0, v1, v2, v3, color}.
 * Winding is counter-clockwise when viewed from outside the cube, so the
 * cross-product normal points outward and back-face culling works. */
static const int faces[N_FACES][5] = {
    {4, 5, 6, 7, COLOR_BLUE},        /* +Z front  */
    {0, 3, 2, 1, COLOR_DARK_BLUE},   /* -Z back   */
    {1, 5, 6, 2, COLOR_RED},         /* +X right  */
    {0, 4, 7, 3, COLOR_DARK_RED},    /* -X left   */
    {3, 7, 6, 2, COLOR_GREEN},       /* +Y top    */
    {0, 1, 5, 4, COLOR_DARK_GREEN},  /* -Y bottom */
};

/* -------------------------------------------------------------------------- */
/* 3D rotation + weak-perspective projection                                  */
/* -------------------------------------------------------------------------- */
static void rotate_xy(double x, double y, double z,
                      double ax, double ay,
                      double *rx, double *ry, double *rz) {
    double cx = pg_cos(ax);
    double sx = pg_sin(ax);
    double cy = pg_cos(ay);
    double sy = pg_sin(ay);

    /* Rotate around Y first. */
    double x1 = x * cy + z * sy;
    double z1 = -x * sy + z * cy;
    /* Then around X. */
    double y2 = y * cx - z1 * sx;
    double z2 = y * sx + z1 * cx;

    *rx = x1;
    *ry = y2;
    *rz = z2;
}

static void project(double x, double y, double z,
                    double *sx, double *sy) {
    double focal = 300.0;
    double scale = focal / (focal + z);
    *sx = x * scale;
    *sy = y * scale;
}

/* Rotated vertices, kept in global storage so the point-cloud rasteriser can
 * access them without threading arrays through every helper call. */
static double rotated[N_VERTICES][3];

/* -------------------------------------------------------------------------- */
/* Point-cloud rasterisation                                                  */
/* -------------------------------------------------------------------------- */
/* Instead of filling faces with scanlines, sample points on each face and
 * draw individual pixels.  POINT_STEP controls the density: larger values
 * produce a coarser, faster "point cloud" look and let us render a much
 * larger cube without choking Scratch. */
static void draw_face_points(int a, int b, int d, double color) {
    double ebx = rotated[b][0] - rotated[a][0];
    double eby = rotated[b][1] - rotated[a][1];
    double ebz = rotated[b][2] - rotated[a][2];

    double edx = rotated[d][0] - rotated[a][0];
    double edy = rotated[d][1] - rotated[a][1];
    double edz = rotated[d][2] - rotated[a][2];

    int n = (int)((2.0 * HALF_SIZE) / POINT_STEP);
    if (n < 2) n = 2;

    for (int i = 0; i <= n; i += 1) {
        double u = (double)i / (double)n;
        for (int j = 0; j <= n; j += 1) {
            double v = (double)j / (double)n;
            double px = rotated[a][0] + u * ebx + v * edx;
            double py = rotated[a][1] + u * eby + v * edy;
            double pz = rotated[a][2] + u * ebz + v * edz;

            double sx, sy;
            project(px, py, pz, &sx, &sy);
            pg_pixel_num(sx, sy, color);
        }
    }
}

/* -------------------------------------------------------------------------- */
/* Main demo: continuously rotating low-resolution point-cloud cube           */
/* -------------------------------------------------------------------------- */
int main(void) {
    pg_init();

    double angle_x = 0.0;
    double angle_y = 0.0;

    while (1) {
        pen_clear();

        for (int i = 0; i < N_VERTICES; i += 1) {
            rotate_xy(vertices[i][0], vertices[i][1], vertices[i][2],
                      angle_x, angle_y,
                      &rotated[i][0], &rotated[i][1], &rotated[i][2]);
        }

        for (int i = 0; i < N_FACES; i += 1) {
            int a = faces[i][0];
            int b = faces[i][1];
            int c = faces[i][2];
            int d = faces[i][3];
            double color = faces[i][4];

            /* Back-face culling: face is visible when its outward normal has
             * a positive Z component (camera looks toward -Z). */
            double ux = rotated[b][0] - rotated[a][0];
            double uy = rotated[b][1] - rotated[a][1];
            double uz = rotated[b][2] - rotated[a][2];
            double vx = rotated[c][0] - rotated[a][0];
            double vy = rotated[c][1] - rotated[a][1];
            double vz = rotated[c][2] - rotated[a][2];
            double nz = ux * vy - uy * vx;

            if (nz > 0.0) {
                /* Draw the visible face as a sparse grid of points. */
                draw_face_points(a, b, d, color);
            }
        }

        angle_x += 0.02;
        angle_y += 0.015;
    }

    return 0;
}
