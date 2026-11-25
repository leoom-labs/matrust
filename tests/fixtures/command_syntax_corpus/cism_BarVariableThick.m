%-------------------------------------------------------------------------
%                                 CISM
%        Modes of a Flexural Bar with Variable Cross Section (EB Beam)
%             CISM Course "Physics of Musical Instruments"
%                          Michele Ducceschi
%                       University of Bologna
%                            10 May 2024
%-------------------------------------------------------------------------
%
% Purpose
% -------
% Compute and visualize the first few mode shapes and (relative) modal
% frequencies of an Euler–Bernoulli bar whose thickness varies along x.
% The bar profile has two flat end sections and a central region with a
% smooth thickness variation of polynomial type.
%
% Mesh
% ----
% An *adaptive* 1D mesh is generated so that the element size h follows
% the local bending wave speed c_phi(x) ~ (E I / rho A)^(1/4). The target
% is 'ppw' points per (highest) wavelength at frequency f_ref.
%
% Governing operator
% ------------------
% We assemble a discrete bilaplacian with variable stiffness:
%     K = D2^T * diag(I(x_i)) * D2     (weak form, 1D)
% with *nonuniform* second-derivative operators D2 and D2^T that enforce
% clamped boundary conditions via the grid-end structure. The mass matrix
% is lumped:  M = diag(rho * A(x_i)).
%
% Eigenproblem
% ------------
% (E * K + εI) * v = (rho * M) * ( (2π f)^2 * v )
% We rewrite as:
%    KM v = λ v,  with  KM = (rho M)^{-1} (E K + εI),  λ = (2π f)^2
% and extract the natural frequencies f and mode shapes v.
%
% Notes
% -----
% - This script plots thickness profiles and selected mode shapes for three
%   bars of increasing taper (different ymin).
% - Uses sparse operators for efficiency and fixes several indexing issues
%   in the original prototype (explicit (i,j) indexing at boundaries).
%-------------------------------------------------------------------------

clear all
close all
clc

for nBar = 1:3
    %----------------------------- Geometry --------------------------------
    ymax   = 4e-2;                % max thickness [m]
    if     nBar == 1, ymin = 3e-2;
    elseif nBar == 2, ymin = 2e-2;
    else               ymin = 0.9e-2;
    end
    wb     = 6e-2;                % width [m]

    xb     = 0.10;                % flat end length on each side [m]
    xw     = 0.50;                % width of varying region [m]
    pexp   = 4;                   % exponent in thickness profile (even)

    E      = 16e9;                % Young's modulus [Pa]
    rho    = 1e3;                 % density [kg/m^3]

    %----------------------- Adaptive mesh controls ------------------------
    adaptivemeshOn = true;
    ppw    = 50;                  % target points per (highest) wavelength
    f_ref  = 1.0e4;               % reference (max) frequency [Hz]
    eps_stab = 1e-9;              % small regularization for K

    %----------------------- Derived / helper quantities -------------------
    L   = 2*xb + xw;              % total length
    xc  = xb + 0.5*xw;            % center of varying region
    Dy  = ymax - ymin;

    Amax = wb * ymax;             % area at maximum thickness
    Imax = (wb * ymax^3) / 12;    % inertia at maximum thickness
    cphi_max = sqrt( sqrt(E*Imax/(rho*Amax)) * (2*pi*f_ref) );
    hmax = 2*pi*cphi_max / (ppw*(2*pi*f_ref));  % target element size

    Amin = wb * ymin;
    Imin = (wb * ymin^3) / 12;
    cphi_min = sqrt( sqrt(E*Imin/(rho*Amin)) * (2*pi*f_ref) );
    hmin = 2*pi*cphi_min / (ppw*(2*pi*f_ref));

    %----------------------------- Mesh build ------------------------------
    % March from x=0 to x=L; update h from local c_phi when adaptive
    x_nodes = 0;
    hvec    = [];
    Acur    = Amax;
    Icur    = Imax;

    x = 0;
    while x < L
        if adaptivemeshOn
            cphi = sqrt( sqrt(E*Icur/(rho*Acur)) * (2*pi*f_ref) );
            h    = max(hmin, min(hmax, 2*pi*cphi/(ppw*(2*pi*f_ref))));
        else
            h    = hmax; % uniform coarse alternative
        end

        x_next = x + h;
        if x_next > L
            h     = L - x;
            x_next= L;
        end

        % Append
        x_nodes = [x_nodes; x_next]; 
        hvec    = [hvec; h];         
        x       = x_next;

        % Update thickness and section for next step
        if (x >= xb) && (x < xb + xw)
            y = ymin + Dy/(0.5*xw)^pexp * abs(x - xc)^pexp;
        else
            y = ymax;
        end
        Icur = (wb*y^3)/12;
        Acur = wb*y;
    end

    % Smoothing h (optional, improves operator quality on strong gradients)
    if numel(hvec) > 1
        hvec(1:end-1) = 0.5*hvec(1:end-1) + 0.5*hvec(2:end);
        hvec(end) = L - sum(hvec(1:end-1));
    end

    % Rebuild nodal arrays for A(x), I(x), thickness t(x)
    M    = numel(x_nodes) - 1;   % number of elements, M+1 nodes
    x    = 0;
    grid = 0;
    Avec = wb*ymax; Ivec = (wb*ymax^3)/12; tvec = ymax;

    for m = 1:M
        x = x + hvec(m);
        grid = [grid; x]; 

        if (x >= xb) && (x < xb + xw)
            y = ymin + Dy/(0.5*xw)^pexp * abs(x - xc)^pexp;
        else
            y = ymax;
        end
        I = (wb*y^3)/12;
        A = wb*y;

        Avec = [Avec; A]; 
        Ivec = [Ivec; I]; 
        tvec = [tvec; y]; 
    end

    % Remove boundary entries for I in interior (used in D2^T * diag(I) * D2)
    I_interior = Ivec(2:end-1);

    %---------------------- Nonuniform 2nd-derivative ----------------------
    % D2: (M-1) x (M+1)
    % For each interior node i (i = 2..M), stencil couples to i-1,i,i+1
    D2  = spalloc(M-1, M+1, 3*(M-1));
    for m = 1:M-1
        hm   = hvec(m);
        hp   = hvec(m+1);
        D2(m, m)     =  2/(hm*(hm+hp));
        D2(m, m+1)   = -2/(hm*hp);
        D2(m, m+2)   =  2/(hp*(hm+hp));
    end

    % D2T: (M+1) x (M-1)   (adjoint structure)
    D2T = spalloc(M+1, M-1, 3*(M-1));
    for m = 1:M-1
        hm = hvec(m);
        hp = hvec(m+1);

        if m == 1
            D2T(1,1) = 1/(hm*hm);  % boundary closure (explicit indexing fix)
        else
            D2T(m,  m) =  2/(hm*(hvec(m-1)+hm));
        end

        D2T(m+1, m) = -2/(hm*hp);

        if m == M-1
            D2T(M+1, M-1) = 1/(hp*hp);  % boundary closure
        else
            D2T(m+2, m) =  2/(hp*(hp + hvec(m+2)));
        end
    end

    %----------------------- Biharmonic stiffness --------------------------
    % biHarm is (M+1)x(M+1)
    biHarm = D2T * spdiags(I_interior, 0, M-1, M-1) * D2;

    % Lumped mass on nodes
    Mmat = spdiags(rho * Avec, 0, M+1, M+1);

    % Generalized eigenproblem -> standard form
    KM   = Mmat \ (E * (biHarm + eps_stab*speye(M+1)));

    [V, Dlam] = eig(full(KM));     
    lam       = diag(Dlam);
    [lam, ind]= sort(lam, 'ascend');
    V         = V(:,ind);

    freqs = sqrt(lam) / (2*pi);    % Hz

    %------------------------------ Plots ----------------------------------
    % Thickness profile
    figure(1)
    if nBar == 1
        plot(grid, tvec/1e-2, 'k', 'LineWidth', 1.2); hold on
    elseif nBar == 2
        plot(grid, tvec/1e-2, 'b', 'LineWidth', 1.2);
    else
        plot(grid, tvec/1e-2, 'r', 'LineWidth', 1.2);
    end
    set(gca,'TickLabelInterpreter','latex','FontSize',13)
    xlabel('$x$ (m)','Interpreter','latex')
    ylabel('$y$ (cm)','Interpreter','latex')
    title('Thickness profile','Interpreter','latex')

    % First few mode shapes (normalized sign)
    figure(2)
    for n = 3:6
        subplot(2,2,n-2)
        phi = V(:,n);
        if phi(1) < 0, phi = -phi; end
        if     nBar == 1, plot(grid, phi, 'k', 'LineWidth',1.2)
        elseif nBar == 2, plot(grid, phi, 'b', 'LineWidth',1.2)
        else               plot(grid, phi, 'r', 'LineWidth',1.2)
        end
        hold on; xlim([0, L])
        set(gca,'TickLabelInterpreter','latex','FontSize',12)
        xlabel('$x$ (m)','Interpreter','latex'); ylabel('$\phi_n(x)$','Interpreter','latex')
        title(sprintf('Mode %d', n), 'Interpreter','latex')
        grid on
    end

    % Relative frequencies wrt the 1st plotted (n=3 here)
    rel = [freqs(3)/freqs(3), freqs(4)/freqs(3), freqs(5)/freqs(3)];
    fprintf('Bar %d relative freqs (n=3 as ref):  [1, %.4f, %.4f]\n', nBar, rel(2), rel(3));

    % Plot relative lines
    figure(3)
    yy = ones(size(grid));
    if     nBar == 1, col = 'k';
    elseif nBar == 2, col = 'b';
    else              col = 'r';
    end
    plot(grid, yy*freqs(3)/freqs(3), col, 'LineWidth',1.2); hold on
    plot(grid, yy*freqs(4)/freqs(3), col, 'LineWidth',1.2)
    plot(grid, yy*freqs(5)/freqs(3), col, 'LineWidth',1.2)
    set(gca,'TickLabelInterpreter','latex','FontSize',12)
    xlabel('$x$ (m)','Interpreter','latex')
    ylabel('$f_n/f_3$','Interpreter','latex')
    title('Relative frequencies (constant lines)','Interpreter','latex')
    grid on
end
