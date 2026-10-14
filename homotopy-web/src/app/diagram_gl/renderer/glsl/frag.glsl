#version 300 es

precision highp float;

uniform vec3 albedo;
uniform bool placeholder;
uniform bool light_stripes;

in float hidden;
in vec3 frag_pos;
in vec3 frag_normal;

layout (location = 0) out vec3 g_position;
layout (location = 1) out vec3 g_normal;
layout (location = 2) out vec4 g_albedo;

void main() {
    if (hidden != 0.) {
        discard;
    }

    // Placeholders are drawn striped (screen-door transparency), which ordinary generators can't
    // imitate. Discarding fragments rather than blending keeps the deferred pipeline unchanged.
    // On a dark colour the gaps would not be seen against it, so the stripes are drawn white
    // there instead (`light_stripes`, from the same rule as the stripes of the SVG views).
    bool stripe = placeholder && mod(gl_FragCoord.x + gl_FragCoord.y, 10.) < 5.;
    if (stripe && !light_stripes) {
        discard;
    }

    vec3 normal = frag_normal;

    if (!gl_FrontFacing) {
        normal = -normal;
    }

    g_position = frag_pos;
    g_normal = normalize(normal);
    // 4th component 1. implies rendered and lit
    g_albedo = vec4(stripe ? vec3(1.) : albedo, 1.);
}
