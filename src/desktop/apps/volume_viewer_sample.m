function [V, L] = vv_shepp_logan()
% Kak & Slaney, Principles of Computerized Tomographic Imaging, Table 3.2 (errata).
% https://engineering.purdue.edu/~malcolm/pct/pct-errata.html
% Columns are centre (x,y,z), semi-axes (A,B,C), rotation about z, and density.
ellipsoids = [
  0     0      0      .6900 .920 .900   0  2.00
  0     0      0      .6624 .874 .880   0 -.98
 -.22   0     -.25    .4100 .160 .210 108 -.02
  .22   0     -.25    .3100 .110 .220  72 -.02
  0     .35   -.25    .2100 .250 .500   0  .02
  0     .10   -.25    .0460 .046 .046   0  .02
 -.08  -.65   -.25    .0460 .023 .020   0  .01
  .06  -.65   -.25    .0460 .023 .020  90  .01
  .06  -.105   .625   .0560 .040 .100  90  .02
  0     .10    .625   .0560 .056 .100   0 -.02
];
q = linspace(-1, 1, 96);
[X, Y] = meshgrid(q, q);
V = zeros(96, 96, 96);
L = uint8(zeros(96, 96, 96));
for k = 1:size(ellipsoids, 1)
    e = ellipsoids(k, :);
    theta = e(7) * pi / 180;
    x = X - e(1);
    y = Y - e(2);
    u = (cos(theta) .* x + sin(theta) .* y) ./ e(4);
    v = (-sin(theta) .* x + cos(theta) .* y) ./ e(5);
    w2 = reshape(((q - e(3)) ./ e(6)).^2, 1, 1, 96);
    mask = (u.^2 + v.^2) + w2 <= 1;
    V = V + e(8) .* double(mask);
    L(mask) = uint8(k);
end
end
