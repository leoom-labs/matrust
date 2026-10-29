% Shared input program; expected outputs are generated independently by MATLAB.
A=reshape(1:24,4,6); v=1:8; w=v'; E=zeros(0,3);
r01=A(:); r02=A(:,:); r03=A(2:4,2:2:end);
r04=A(end:-1:1); r05=A([4 1 4],[6 1 6]);
r06=A([1 2;7 8]); r07=v([2;1]); r08=w([2 1]);
r09=v([1 2;3 4]); r10=w([1 2;3 4]);
r11=A([],2); r12=A(2,[]); r13=A([]); r14=v([]); r15=w([]);
r16=A(3:1,:); r17=A(:,4:2); r18=E(:); r19=E(:,:);
r20=A(3:0.25:3); r21=A(1.5:1); r22=A(1:2:6);
r23=A(end-2:end); r24=A([end 1],end-1:end);
r25=A(v(1:end-4),w(end-2:-1:end-4));
r26=A(2:2:4,6:-2:1); r27=A(4:-3:1,1:3:6);
r28=A(zeros(0,2)); r29=A(zeros(2,0));
A=reshape(1:65536,8192,8);
r30=A(:,2:7); r31=A(1:2:end,:); r32=A(end:-1:1);
r33=A([8192 1 8192 7],8:-2:2);
