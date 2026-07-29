#version 150

layout(std140) uniform window_info {
	mat4 transform;
	float alphaBlend;
};

in vec3 position;
in vec2 uv;

out vec2 texCoord;

void main() {
	gl_Position = transform * vec4(position, 1.0);
	texCoord = uv;
}