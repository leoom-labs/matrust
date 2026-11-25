%% Calcolo Numerico
% Esercitazione 3
% Kevin Michael Frick

format compact;

%% Metodo delle potenze

A1 = [0 0 1 1; 1 0 0 0; 1 1 0 1; 1 1 0 0];
A2 = [0 1 0 1 1; 0 0 1 1 1; 1 0 0 1 0; 0 0 0 0 1; 0 0 1 0 0];

x1 = PowerMethod(A1, ones(length(A1), 1), 20);
r1 = x1 ./ norm(x1, 1)
[x1_m, D] = eigs(A1, 1);
r1_m = x1_m ./ norm(x1_m, 1)
err_1 = norm(r1 - r1_m) / norm(r1_m)

x2 = PowerMethod(A2, ones(length(A2), 1), 20);
r2 = x2 ./ norm(x2, 1)
[x2_m, D] = eigs(A2, 1);
r2_m = x2_m ./ norm(x2_m, 1)
err_2 = norm(r2 - r2_m) / norm(r2_m)

%% SVD

image = cell(3);
image{1} = rgb2gray(im2double(imread("img1.jpg")));
image{2} = im2double(imread("img2.jpg"));
image{3} = rgb2gray(im2double(imread("img3.jpg")));
image_largest = cell(3);
image_smallest = cell(3);
image_k = cell(3);
image_k_val = cell(3);
image_err = cell(3);
sigma = cell(3);

for i = 1:3
    [image_largest{i}, image_smallest{i}, image_k{i}, image_k_val{i}, image_err{i}, sigma{i}] = SVD(image{i});
end
for i = 1:3
    fprintf("Image %i\n", i);
    PlotSVD(image{i}, image_err{i}, sigma{i});
    %PrintSVD(image_largest{i}, image_smallest{i}, image_k{i}, image_k_val{i});
end

%% Least squares

pts2 = [0.11019 150000;
         0.21956        300000;
         0.32949        450000;
         0.43899        600000;
        0.54803        750000;
         0.65694        900000;
         0.76562       1050000;
         0.87487       1200000;
         0.98292       1350000;
        1.09146       1500000;
        1.20001       1650000;
        1.30822       1800000;
        1.41599       1950000;
        1.52399       2100000;
        1.63194       2250000;
        1.73947       2400000;
        1.84646       2550000;
        1.95392       2700000;
        2.06128       2850000;
        2.16844       3000000;
         0.11052        150000;
         0.22018        300000;
         0.32939        450000;
         0.43886        600000;
         0.54798        750000;
         0.65739        900000;
         0.76596       1050000;
         0.87474       1200000;
         0.98300       1350000;
        1.09150       1500000;
        1.20004       1650000;
        1.30818       1800000;
        1.41613       1950000;
        1.52408       2100000;
        1.63159       2250000;
        1.73965       2400000;
        1.84696       2550000;
        1.95445       2700000;
        2.06177       2850000;
        2.16829       3000000];

pts = [1.0 1.18; 1.2 1.26; 1.4 1.23; 1.6 1.37; 1.8 1.37; 2.0 1.45; 2.2 1.42; 2.4 1.46; 2.6 1.53; 2.8 1.59; 3.0 1.50];

[alpha, alpha_svd] = PolApprox(pts);
PlotPolApprox(pts, alpha, alpha_svd);
[alpha, alpha_svd] = PolApprox(pts2);
PlotPolApprox(pts2, alpha, alpha_svd);
    
