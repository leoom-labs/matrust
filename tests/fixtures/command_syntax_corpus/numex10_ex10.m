% Tutorial Minggu ke-9
% MENCARI SOLUSI NUMERIK SISTEM PERSAMAAN DIFERENSIAL
% 
% File ini akan memanggil beberapa kode dan anda harus melengkapi kode tersebut
% untuk mencari hampiran turunan beberapa fungsi
% 
% File latihan :    
%     1. myode.m
%     2. rk4.m
%     3. OdeIntegration.m
%     4. myode2.m
% 	  5. rk2sistem.m	
%     
% Pada latihan ini, anda tidak perlu mengotak-atik code pada file ini 
% anda cukup membuat kode program pada file-file diatas.
% 
% 
%==========================================================================
% PART 1: MENDEFINISIKAN PERSAMAAN DIFERENSIAL YANG AKAN DICARI SOLUSINYA
% Buatlah sebuah fungsi yang diberi nama myode.m yang isinya adalah fungsi
% yang akan dicari integralnya.
% Bualah kode yang merepresentasikan fungsi dy/dt = f(y) = -y^3+3y

clc;
clear;
close all;
t = 0;
y = [-5:0.1:5];
fprintf('Part 1 : Mendefinisikan pdb yang akan dicari solusinya \n');
dy = myode(t,y);
figure(1);
plot(y,dy);
xlabel ("y");
ylabel ("dy");
fprintf('Kode yang anda kerjakan adalah benar jika muncul sebuah window baru  \n');
fprintf('yang menunjukkan grafik y terhadap dy \n'); 
fprintf('Tekan tombol apapun untuk lanjut \n\n');
pause;
close (1); %close figure(1)

%==========================================================================
% PART 2: MENCARI SOLUSI NUMERIK PDB dy/dt=f(t,y) DENGAN METODE RUNGE KUTTA ORDE 4
% Buatlah sebuah fungsi yang diberi nama rk4.m yang akan mencari solusi
% numerik suatu pdb dengan metode Runge Kutta orde 4

fprintf('Part 2 : Mencari solusi numerik PDB dengan metode Runge Kutta orde 4 \n');
t_init = 0; %initial condition for t
t_end = 5; %batas atas integrasi
y_init = 1; %initial condition for y
h = 0.25; %lebar selang / step size
[T,Y] = rk4(@(t,y)myode(t,y),t_init,t_end,y_init,h);
Y(end)
fprintf('Kode yang anda kerjakan adalah benar jika element terakhir dari vektor Y adalah \n');
fprintf('1.7321 \n');
fprintf('Tekan tombol apapun untuk lanjut\n\n');
pause;

%==========================================================================
% PART 3: MEMBANDINGKAN SOLUSI SEBENARNYA DENGAN SOLUSI NUMERIK
% Anda tidak perlu melakukan apa-apa di bagian ini. Pada bagian ini akan dipanggil
% sebuah fungsi yang akan membandingkan solusi sebenarnya dengan solusi numerik

fprintf('Part 3 : Membandingkan solusi numerik dengan solusi sebenarnya \n');
%----------------------------------------------------------------
%This step needs to be changed if you consider a different ODE
real_sol = @(t) (3./sqrt(3+6*exp(-6*t))) ; %this is the real solution
%----------------------------------------------------------------
Y_real = real_sol(T);
compareplot(T,Y,Y_real)
fprintf('Tekan tombol apapun untuk lanjut\n\n');
pause;
close (1); %close figure(1)

%==========================================================================
% PART 4: MENGGAMBARKAN SOLUSI NUMERIK PDB UNTUK BERBAGAI NILAI INITIAL CONDITION
% Buatlah sebuah fungsi void yang diberi nama OdeIntegration.m yang akan mencari
% solusi numerik suatu PDB untuk berbagai nilai awal. Masing-masing solusi numerik
% akan diplot dalam satu grafik yang sama

fprintf('Part 4 : Mencari solusi numerik PDB untuk berbagai nilai initial condition\n');
t_init = 0; %initial condition for t
t_end = 5; %batas atas integrasi
y_init = [-1 -0.1 0 0.1 1]; %initial condition for y
h = 0.05; %lebar selang / step size
OdeIntegration(@(t,y)myode(t,y),t_init,t_end,y_init,h);
fprintf('Kode yang anda kerjakan adalah benar jika muncul sebuah outputnya sebuah gambar yang \n');
fprintf('terdiri dari 5 grafik yang menunjukan solusi numerik untuk masing-masing initial condition. \n');
fprintf('Tekan tombol apapun untuk lanjut\n\n');
pause;


%==========================================================================
% PART 5: MENDEFINISIKAN SISTEM PERSAMAAN DIFERENSIAL YANG AKAN DICARI SOLUSINYA
% Buatlah sebuah fungsi yang diberi nama myode2.m yang isinya adalah fungsi
% yang akan dicari integralnya.
% Buatlah kode yang merepresentasikan sistem pdb berikut ini 
% dx/dt = x(15-3y)
% dy/dt = y(-4+2x)

fprintf('Part 5 : Mendefinisikan sistem pdb yang akan dicari solusinya \n');
t = 0;
y = [1 1];
dy = myode2(t,y)
fprintf('Kode yang anda kerjakan adalah benar jika outputnya adalah \n');
fprintf('12 -2 \n'); 
fprintf('Tekan tombol apapun untuk lanjut \n\n');
pause;

%==========================================================================
% PART 6: MENCARI SOLUSI NUMERIK SISTEM PDB DUA DIMENSI DENGAN METODE RUNGE KUTTA ORDE 2
% Buatlah sebuah fungsi yang diberi nama rk2sistem.m yang akan mencari solusi
% numerik suatu sistem pdb dengan metode Runge Kutta orde 2

fprintf('Part 6 : Mencari solusi numerik sistem PDB dua dimensi dengan metode Runge Kutta orde 2 \n');
t_init = 0; %initial condition for t
t_end = 5; %batas atas integrasi
y_init = [4 5]; %initial condition for y
h = 0.1; %lebar selang / step size
[T,Y] = rk2sistem(@(t,y)myode2(t,y),t_init,t_end,y_init,h);
Y(end,:)
fprintf('Kode yang anda kerjakan adalah benar jika element terakhir dari vektor Y adalah \n');
fprintf(' 1.6036   7.5850\n');
plot(Y(:,1),Y(:,2));
fprintf('Program Selesai. Tekan tombol apapun untuk lanjut\n\n');
pause;
close all;


% ==============================AKHIR KODE================================
% TIDAK MELAKUKAN PENGETIKAN KODE DI BAWAH GARIS INI AGAR LEBIH RAPI DALAM
% MELAKUKAN KODING DAN LEBIH DIMENGERTI.

% BONUS : 
% 1. COBALAH UNTUK MENYELESAIKAN PDB YANG LAIN 
% 
% 2. COBALAH UNTUK MENGGANTI NILAI h YANG LEBIH KECIL, APA YANG TERJADI DENGAN SOLUSI SISTEM PDB
%
% 3. COBALAH UNTUK MEMODIFIKASI FILE RK4.m AGAR DAPAT DIGUNAKAN UNTUK MENCARI SOLUSI PADA SISTEM PDB


