function R = user_function_cases
    A = [1 2 3];
    [changed,mask] = changed_input(A);
    a = optional(5);
    [b,ni,no] = optional(5,3);
    [~,second] = pair(7);
    ignored = ignore_input(1,99,3);
    mode();
    zero_outputs = ans;
    [nested,outer_outputs] = nested_context(4);
    R = [A changed double(mask) a b ni no second ignored zero_outputs nested outer_outputs factorial(6) early(-4) clear_local(8)];
end
function [y,m] = changed_input(x)
    x(1) = 9;
    y = x;
    m = x>2;
end
function [y,ni,no] = optional(x,factor)
    ni = nargin;
    no = nargout;
    if nargin<2
        factor = 2;
    end
    y = x*factor;
end
function [a,b] = pair(x)
    a = x;
    b = x+1;
end
function y = ignore_input(a,~,b)
    y = a+b+nargin;
end
function y = mode
    y = nargout;
end
function [y,n] = nested_context(x)
    y = optional(x);
    n = nargout;
end
function y = factorial(n)
    if n<=1
        y = 1;
    else
        y = n*factorial(n-1);
    end
end
function y = early(x)
    y = x;
    if x<0
        return;
    end
    y = x+1;
end
function y = clear_local(x)
    clear;
    y = 3;
end
