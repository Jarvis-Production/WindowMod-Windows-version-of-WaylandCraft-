#version 150

layout(std140) uniform window_info {
	mat4 transform;
	float alphaBlend;
};

uniform sampler2D sampler;

in vec2 texCoord;

out vec4 fragColor;

void main() {
	vec4 color = texture(sampler, texCoord);
	color.a = color.a + alphaBlend * (1 - color.a);
	if(color.a == 0.0) {
		discard;
	}
	fragColor = color;
}