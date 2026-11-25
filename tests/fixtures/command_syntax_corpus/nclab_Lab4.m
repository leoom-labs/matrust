%% Calcolo Numerico
% Esercitazione 4
% Kevin Michael Frick

%% Metodo di bisezione e di Newton

f = @(x) exp(x) - power(x, 2);
df = @(x) exp(x) - 2 * x;
cond_num = @(x) norm(1 / df(x));

iter_max = 10;
x_sol = -0.7034674;
[bisection_x, bisection_err, bisection_f] = Bisection(f, -1, 1, x_sol, iter_max);
[newton_x, newton_err, newton_f] = Newton(f, df, 1, x_sol, iter_max);
figure;
hold on;
title("Root finding");
xlabel("Iteration");
ylabel("Relative error");
plot(1:iter_max, bisection_err);
plot(1:iter_max, newton_err);
legend("Bisection", "Newton");
hold off;
arr = -1:0.01:1;
figure;
hold on;
title("Condition number of f(x) = exp(x) - x^2");
plot(arr, arrayfun(cond_num,arr));
hold off;

figure;
hold on;
title("Convergence");
xlabel("Iteration");
ylabel("f(x)");
plot(bisection_f);
plot(newton_f);
legend("Bisection", "Newton");
hold off;

figure;
hold on;
title("Convergence");
xlabel("Iteration");
ylabel("x");
plot(bisection_x);
plot(newton_x);
legend("Bisection", "Newton");
hold off;



%% Discesa del gradiente

f = @(x, y) 100 * (y - x .^ 2) .^2 + (1 - x) .^ 2;
[X, Y] = meshgrid(-2:0.2:2);
Z = f(X, Y);

surf(X, Y, Z);


f = @(x) 100 * (x(2) - x(1) .^ 2) .^2 + (1 - x(1)) .^ 2;
df = @(x) [100 * 2 * (x(2) - x(1) .^ 2) * (-2 * x(1)) - 2 * (1 - x(1)) 100 * 2 * (x(2) - x(1) .^ 2)];
iter_max = 1000;

% Errore relativo e passo
[x_star, err, alpha, ~] = GradientDescent(f, df, [-0.8 -0.8], [1 1], iter_max, 1e-2);
figure;
hold on;
title("Gradient descent - Relative error");
xlabel("Iteration");
ylabel("Relative error");
plot(err);
hold off;
figure;
hold on;
title("Gradient descent - Descent rate (moving average)");
xlabel("Iteration");
ylabel("Rate");
plot(movmean(alpha, length(alpha) / 10));
hold off;

iter = 0;
% Numero di iterazioni variando il punto iniziale
iter_max = 100000;
i = 1;
arr = -10:1:10;
for k = arr
    j = [k k];
    [~, ~, ~, iter(i)] = GradientDescent(f, df, j, [1 1], iter_max, 1e-6);
    i = i + 1;
    
end
figure;
hold on;
title(["Gradient descent iterations required"; "Starting from differents points along the y = x line"]);
xlabel("Point coordinates");
ylabel("Iterations");
plot(arr, iter);
hold off;

iter = 0;

% Numero di iterazioni variando il criterio di tolleranza
iter_max = 10000;
i = 1;
arr = -13:-1;
for k = arr
    [~, ~, ~, iter(i)] = GradientDescent(f, df, [0.5 0.5], [1 1], iter_max, power(10, k));
    i = i + 1;
end
figure;
hold on;
title(["Gradient descent iterations required"; "Varying the tolerance criterium"]);
xlabel("Log tolerance");
ylabel("Iterations required");
plot(arr, iter);
hold off;


%% Interpolazione della funzione di Runge

f = @(x) 1 ./ (1 + power(x, 2));

a = -5;
b = 5;
len = b - a;
nmin = 7;
nmax = 13;
int_plot = a:0.01:b;


figure;
hold on;
grid on;
title(["Runge function"; "Lagrange interpolation on uniformly spaced points"]);
plot (int_plot, f(int_plot));

LegendStrings = cell(1, nmax - nmin - 1);
i = 1;
LegendStrings{i} = "Runge function";
for n = nmin:2:nmax
    i = i + 1;
    int_interpolate = a:(len / n): b;
    Lagr = @(x) Lagrange(int_interpolate, f(int_interpolate), x);
    plot(int_plot, arrayfun(Lagr, int_plot));
    LegendStrings{i} = ['n = ',num2str(n)];
end
legend(LegendStrings);
hold off;

figure;
hold on;
grid on;
title(["Runge function"; "Lagrange interpolation on Chebyshev nodes"]);
plot (int_plot, f(int_plot));

LegendStrings = cell(1, nmax - nmin - 1);
i = 1;
LegendStrings{i} = "Runge function";
for n = nmin:2:nmax
    i = i + 1;
    int_interpolate = ChebyshevNodes(a, b, n);
    Lagr = @(x) Lagrange(int_interpolate, f(int_interpolate), x);
    plot(int_plot, arrayfun(Lagr, int_plot));
    LegendStrings{i} = ['n = ',num2str(n)];
end
legend(LegendStrings);
hold off;
