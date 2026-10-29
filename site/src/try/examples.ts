export const homeExample = `A = [3 1; 1 2];
b = [9; 8];
x = A \\ b;
disp(x)
t = linspace(0, 2*pi, 100);
plot(t, sin(t))`;
export const examples = [
  { name: 'Basics', code: `x = 1:5;\ny = x.^2;\ndisp(y)\ndisp(sum(y))` },
  { name: 'Matrices and solving', code: homeExample },
  { name: 'Plotting', code: `t = linspace(0, 2*pi, 100);\nplot(t, sin(t))\nxlabel('t')\nylabel('sin(t)')\ngrid on` },
  { name: 'Loops and function handles', code: `square = @(x) x.^2;\nfor k = 1:4\n    disp(square(k))\nend` },
  { name: 'Strings, cells and structs', code: `label = 'MatRust';\nC = {label, [1 2 3]};\ns = struct('name', C{1}, 'values', C{2});\ndisp(s.name)\ndisp(s.values)` },
  { name: 'A small signal', code: `t = linspace(0, 1, 200);\nx = sin(2*pi*5*t) + 0.25*sin(2*pi*20*t);\ny = filter([1 1 1]/3, 1, x);\nplot(t, y)\nxlabel('Time (s)')\ngrid on` },
];
