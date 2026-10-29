% Solve A*x = b without explicitly forming the inverse.
A = [4 1 2; 1 5 1; 2 1 6];
b = [7; 8; 9];
x = A \ b
residual = norm(A*x - b)
d = det(A)
